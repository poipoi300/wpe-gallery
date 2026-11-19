use wpe_gallery::{backend, library, renderer, safety};
mod grid;
mod preview;
mod surface;
mod title;
mod ui;
mod verification;
use anyhow::Context as _;
use backend::ApplyBackend;
use futures::StreamExt as _;
use gpui::*;
use gpui_component::{Root, VirtualListScrollHandle};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};
use verification::Verification;
struct Gallery {
    items: Vec<library::Wallpaper>,
    outputs: Vec<backend::Output>,
    output: usize,
    selected: usize,
    status: String,
    busy: bool,
    pool: preview::Pool,
    thumbnails: HashMap<usize, Thumbnail>,
    // In-flight thumbnail decodes. Dropping a task cancels its queued decode.
    thumbnail_tasks: HashMap<usize, Task<()>>,
    // Indices the grid asked for since the last render; decodes for anything
    // else (scrolled away) are cancelled at the next render.
    requested: HashSet<usize>,
    // Render counter; thumbnails record the last frame that used them.
    frame: u64,
    // Frames of the selected preview and the one on screen.
    large: preview::Frames,
    large_frame: usize,
    generation: usize,
    large_task: Option<Task<()>>,
    animation: Option<Task<()>>,
    cache_limit: usize,
    // Tile under the pointer and when it got there (drives the title marquee).
    hovered: Option<(usize, Instant)>,
    // The one pending repaint at the end of a marquee pause.
    marquee_wake: title::Wake,
    review_open: bool,
    scroll: VirtualListScrollHandle,
    row_sizes: std::rc::Rc<Vec<Size<Pixels>>>,
    scroll_selection: bool,
    last_columns: usize,
    focus: FocusHandle,
    safety: Vec<safety::Status>,
    visible: Vec<usize>,
    safe_only: bool,
    safety_task: Option<Task<()>>,
    recheck_tasks: HashMap<usize, Task<()>>,
    verification: Verification,
    #[cfg(test)]
    renders: usize,
}
struct Thumbnail {
    image: Option<Arc<RenderImage>>,
    light: surface::EdgeLight,
    used: u64,
}
/// Messages from the verification worker to the gallery.
enum Update {
    Item(usize, safety::Status),
    Complete,
    Failed(String),
}
actions!(gallery, [Quit, Apply, Next, Previous, Up, Down, NextOutput]);
/// Check every item against the whitelist on a few background threads,
/// reusing cached hashes of unchanged trees. Results are streamed so progress
/// and newly verified tiles appear as they finish.
fn verify_all(
    items: Vec<(String, PathBuf)>,
    sender: futures::channel::mpsc::UnboundedSender<Update>,
) {
    let path = safety::whitelist_path();
    let reviews = match safety::read_whitelist(&path) {
        Ok(reviews) => reviews,
        Err(error) => {
            let _ = sender.unbounded_send(Update::Failed(format!(
                "Could not load whitelist {}: {error:#}",
                path.display()
            )));
            return;
        }
    };
    let roots = library::libraries();
    let cache = safety::HashCache::shared();
    let mut open = true;
    safety::parallel_map(
        &items,
        |(id, directory)| safety::assess_installed(&reviews, id, directory, &roots, Some(cache)),
        |index, status| {
            open = sender.unbounded_send(Update::Item(index, status)).is_ok();
            open
        },
    );
    if open {
        let installed: std::collections::HashSet<&str> =
            items.iter().map(|(id, _)| id.as_str()).collect();
        cache.retain(|id| installed.contains(id));
    }
    if let Err(e) = cache.flush() {
        log::warn!("Cannot save hash cache: {e:#}");
    }
    if open {
        let _ = sender.unbounded_send(Update::Complete);
    }
}
impl Gallery {
    fn new(
        items: Vec<library::Wallpaper>,
        outputs: Vec<backend::Output>,
        initial: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected = initial
            .or_else(|| {
                let config = library::load_config(&library::config_path()).ok()?;
                let id = &config.outputs.get(&outputs.first()?.name)?.wallpaper_id;
                items.iter().position(|i| &i.id == id)
            })
            .unwrap_or(0);
        let count = items.len();
        Self {
            items,
            outputs,
            selected,
            output: 0,
            status: String::new(),
            busy: false,
            pool: preview::Pool::new(),
            thumbnails: HashMap::new(),
            thumbnail_tasks: HashMap::new(),
            requested: HashSet::new(),
            frame: 0,
            large: Vec::new(),
            large_frame: 0,
            generation: 0,
            large_task: None,
            animation: None,
            cache_limit: 256,
            hovered: None,
            marquee_wake: title::Wake::default(),
            review_open: false,
            scroll: VirtualListScrollHandle::new(),
            row_sizes: Default::default(),
            scroll_selection: true,
            last_columns: 0,
            focus: cx.focus_handle(),
            safety: vec![safety::Status::unreviewed("Checking reviews…"); count],
            visible: Vec::new(),
            safe_only: true,
            safety_task: None,
            recheck_tasks: HashMap::new(),
            verification: Verification::Checking {
                completed: 0,
                total: count,
            },
            #[cfg(test)]
            renders: 0,
        }
    }
    fn rebuild_visible(&mut self, cx: &mut Context<Self>) {
        let was_shown = self.visible.contains(&self.selected);
        self.visible = (0..self.items.len())
            .filter(|i| !self.safe_only || self.safety[*i].allowed)
            .collect();
        let shown = self.visible.contains(&self.selected);
        if !shown {
            self.large_task.take();
            self.drop_large(cx);
            // While verification runs, the requested selection (saved
            // assignment or --select) may still become visible; only fall
            // back to the first item once every result is in.
            if !matches!(self.verification, Verification::Checking { .. })
                && let Some(index) = self.visible.first().copied()
            {
                self.select(index, cx);
            }
        } else if !was_shown {
            // The requested selection just appeared: load its preview and
            // bring it into view once.
            self.select(self.selected, cx);
        }
        cx.notify();
    }
    fn refresh_safety(&mut self, cx: &mut Context<Self>) {
        self.recheck_tasks.clear();
        self.verification = Verification::Checking {
            completed: 0,
            total: self.items.len(),
        };
        self.safety = vec![safety::Status::unreviewed("Checking reviews…"); self.items.len()];
        self.rebuild_visible(cx);
        let items = self
            .items
            .iter()
            .map(|i| (i.id.clone(), i.directory.clone()))
            .collect();
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        let worker = cx
            .background_executor()
            .spawn(async move { verify_all(items, sender) });
        self.listen(receiver, Some(worker), cx);
    }
    /// Apply verification results on the foreground executor. The task is
    /// owned by this entity; each batch updates it through its weak handle
    /// and notifies it, which invalidates the window rendering it.
    fn listen(
        &mut self,
        updates: impl futures::Stream<Item = Update> + Unpin + 'static,
        worker: Option<Task<()>>,
        cx: &mut Context<Self>,
    ) {
        let mut batches = updates.ready_chunks(256);
        self.safety_task = Some(cx.spawn(async move |this, cx| {
            let _worker = worker;
            while let Some(batch) = batches.next().await {
                let applied = this.update(cx, |this, cx| {
                    for update in batch {
                        this.apply_update(update);
                    }
                    this.rebuild_visible(cx);
                });
                if applied.is_err() {
                    break;
                }
            }
        }));
    }
    fn apply_update(&mut self, update: Update) {
        match update {
            Update::Item(index, status) => {
                self.safety[index] = status;
                if let Verification::Checking { completed, .. } = &mut self.verification {
                    *completed += 1;
                }
            }
            Update::Complete => {
                self.verification = Verification::Ready;
                // Earlier results inserted tiles above the selection.
                self.scroll_selection = true;
                eprintln!(
                    "Whitelist verification complete: {} of {} wallpapers safe",
                    self.safety.iter().filter(|s| s.allowed).count(),
                    self.items.len()
                );
            }
            Update::Failed(error) => {
                eprintln!("{error}");
                self.verification = Verification::Failed(error);
            }
        }
    }
    /// Re-read the whitelist and fully re-hash one item, bypassing the hash
    /// cache, e.g. after Apply refused it.
    fn recheck(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(index) else {
            return;
        };
        let (id, directory) = (item.id.clone(), item.directory.clone());
        let job = cx.background_executor().spawn(async move {
            // The cached record may be what showed a stale Safe badge; drop it
            // so the next verification pass rehashes this item too.
            let cache = safety::HashCache::shared();
            cache.retain(|cached| cached != id);
            if let Err(e) = cache.flush() {
                log::warn!("Cannot save hash cache: {e:#}");
            }
            match safety::read_whitelist(&safety::whitelist_path()) {
                Ok(reviews) => {
                    safety::assess_installed(&reviews, &id, &directory, &library::libraries(), None)
                }
                Err(e) => safety::Status::unreviewed(format!("Could not load whitelist: {e:#}")),
            }
        });
        let task = cx.spawn(async move |this, cx| {
            let status = job.await;
            _ = this.update(cx, |this, cx| {
                this.recheck_tasks.remove(&index);
                this.safety[index] = status;
                this.rebuild_visible(cx);
            });
        });
        self.recheck_tasks.insert(index, task);
    }
    fn move_selection(&mut self, step: grid::Step, cx: &mut Context<Self>) {
        if self.visible.is_empty() {
            return;
        }
        let position = self
            .visible
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0);
        let position = grid::step(position, self.visible.len(), self.last_columns, step);
        self.choose(self.visible[position], cx);
    }
    /// A selection the user made: status sits right above Apply, so feedback
    /// about the previous item would read as if it concerned this one.
    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.busy {
            self.status.clear();
        }
        self.select(index, cx);
    }
    fn drop_large(&mut self, cx: &mut Context<Self>) {
        self.animation.take();
        self.large_frame = 0;
        for (image, _) in self.large.drain(..) {
            cx.drop_image(image, None);
        }
    }
    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.large_task.take();
        self.selected = index;
        self.scroll_selection = true;
        self.generation += 1;
        self.drop_large(cx);
        let generation = self.generation;
        if let Some(path) = self.items.get(index).and_then(|i| i.preview.clone())
            && let Some(receiver) = self.pool.request(path, true)
        {
            self.large_task = Some(cx.spawn(async move |this, cx| {
                if let Ok(result) = receiver.await {
                    _ = this.update(cx, |this, cx| {
                        if this.generation == generation {
                            match result {
                                Ok(frames) => this.animate_large(frames, cx),
                                Err(e) => this.status = format!("Preview unavailable: {e}"),
                            }
                            cx.notify();
                        }
                    });
                }
            }));
        }
        cx.notify();
    }
    /// Advance an animated selected preview at its own frame delays, so the
    /// window repaints at the GIF's rate rather than every display refresh.
    fn animate_large(&mut self, frames: preview::Frames, cx: &mut Context<Self>) {
        self.drop_large(cx);
        let delays: Vec<_> = frames.iter().map(|(_, delay)| *delay).collect();
        self.large = frames;
        if delays.len() < 2 {
            return;
        }
        self.animation = Some(cx.spawn(async move |this, cx| {
            for index in (0..delays.len()).cycle() {
                cx.background_executor().timer(delays[index]).await;
                let advanced = this.update(cx, |this, cx| {
                    this.large_frame = (index + 1) % delays.len();
                    cx.notify();
                });
                if advanced.is_err() {
                    break;
                }
            }
        }));
    }
    fn thumb(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        self.requested.insert(index);
        if let Some(thumbnail) = self.thumbnails.get_mut(&index) {
            thumbnail.used = self.frame;
            return thumbnail.image.clone();
        }
        if !self.thumbnail_tasks.contains_key(&index)
            && let Some(path) = self.items[index].preview.clone()
            && let Some(receiver) = self.pool.request(path, false)
        {
            let task = cx.spawn(async move |this, cx| {
                if let Ok(result) = receiver.await {
                    _ = this.update(cx, |this, cx| {
                        this.thumbnail_tasks.remove(&index);
                        let image = result.ok().and_then(|frames| frames.into_iter().next());
                        this.thumbnails.insert(
                            index,
                            Thumbnail {
                                light: image
                                    .as_ref()
                                    .map(|(image, _)| surface::EdgeLight::sample(image))
                                    .unwrap_or_default(),
                                image: image.map(|(image, _)| image),
                                used: this.frame,
                            },
                        );
                        this.evict_thumbnails(cx);
                        cx.notify();
                    });
                }
            });
            self.thumbnail_tasks.insert(index, task);
        }
        None
    }
    /// Drop the least recently drawn thumbnails beyond the cache limit.
    fn evict_thumbnails(&mut self, cx: &mut Context<Self>) {
        let excess = self.thumbnails.len().saturating_sub(self.cache_limit);
        if excess == 0 {
            return;
        }
        let mut by_age: Vec<_> = self.thumbnails.iter().map(|(i, t)| (t.used, *i)).collect();
        by_age.select_nth_unstable(excess - 1);
        for (_, index) in by_age.into_iter().take(excess) {
            if let Some(Thumbnail {
                image: Some(image), ..
            }) = self.thumbnails.remove(&index)
            {
                cx.drop_image(image, None);
            }
        }
    }
    /// Cancel decodes for tiles the last frame no longer displayed.
    fn cancel_offscreen_thumbnails(&mut self) {
        let requested = std::mem::take(&mut self.requested);
        if !requested.is_empty() {
            self.thumbnail_tasks
                .retain(|index, _| requested.contains(index));
        }
    }
    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(output) = self.outputs.get(self.output) else {
            self.status = "No connected output. Reopen after connecting a display.".into();
            cx.notify();
            return;
        };
        let Some(item) = self.items.get(self.selected) else {
            return;
        };
        if !self.safety[self.selected].allowed {
            self.status = format!("Apply refused: {}", self.safety[self.selected].reason);
            cx.notify();
            return;
        }
        let (output, id, index) = (output.name.clone(), item.id.clone(), self.selected);
        self.busy = true;
        self.status = "Applying wallpaper…".into();
        cx.notify();
        // Platform::apply re-verifies the whitelist and hash itself; the
        // gallery's status is only a hint and never authorizes Apply.
        let job = cx.background_executor().spawn(async move {
            backend::Platform
                .apply(&output, &id)
                .map(|_| output)
                .map_err(|e| format!("{e:#}"))
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                if result.is_err() {
                    this.recheck(index, cx);
                }
                this.status = match result {
                    Ok(o) => format!("Applied to {o}"),
                    Err(e) => e,
                };
                cx.notify();
            });
        })
        .detach();
    }
}
impl Gallery {
    /// Keep decodes for the rows just outside the viewport, so thumbnails are
    /// usually ready by the time a row scrolls in.
    fn prefetch(&mut self, rows: std::ops::Range<usize>, columns: usize, cx: &mut Context<Self>) {
        let start = rows.start.saturating_sub(1) * columns;
        let end = ((rows.end + 2) * columns).min(self.visible.len());
        for position in start..end {
            self.thumb(self.visible[position], cx);
        }
    }
}
// GPUI and its renderer report swapchain/present problems through `log`.
// WPE_GALLERY_LOG=info|warn|debug prints them to stderr (default: warn).
struct StderrLogger;
impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }
    }
    fn flush(&self) {}
}
fn init_logging() {
    let level = std::env::var("WPE_GALLERY_LOG")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(log::LevelFilter::Warn);
    if log::set_logger(&StderrLogger).is_ok() {
        log::set_max_level(level);
    }
}
fn main() -> anyhow::Result<()> {
    init_logging();
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--content-hash") {
        let path = args
            .get(2)
            .context("usage: wpe-gallery --content-hash ITEM_DIRECTORY")?;
        println!("{}", safety::content_hash(std::path::Path::new(path))?);
        return Ok(());
    }
    if args.get(1).is_some_and(|a| a == "--launch-renderer") {
        return renderer::launch();
    }
    if args.get(1).is_some_and(|a| a == "--renderer-args") {
        println!("{}", serde_json::to_string(&renderer::arguments(false)?)?);
        return Ok(());
    }
    let (items, warnings) = library::scan();
    let outputs = backend::outputs();
    if args.iter().any(|a| a == "--scan") {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"wallpapers":items,"outputs":outputs,"warnings":warnings})
            )?
        );
        return Ok(());
    }
    let initial = args
        .windows(2)
        .find(|a| a[0] == "--select")
        .and_then(|a| items.iter().position(|i| i.id == a[1]));
    eprintln!(
        "Loaded {} wallpapers; {} outputs; {} skipped items",
        items.len(),
        outputs.len(),
        warnings.len()
    );
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            ui::init(cx);
            cx.bind_keys([
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("enter", Apply, Some("Gallery")),
                KeyBinding::new("right", Next, Some("Gallery")),
                KeyBinding::new("left", Previous, Some("Gallery")),
                KeyBinding::new("up", Up, Some("Gallery")),
                KeyBinding::new("down", Down, Some("Gallery")),
                KeyBinding::new("tab", NextOutput, Some("Gallery")),
            ]);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1280.), px(840.)),
                        cx,
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("WPE Gallery".into()),
                        ..Default::default()
                    }),
                    app_id: Some("io.github.poipoi300.wpe-gallery".into()),
                    window_min_size: Some(size(px(760.), px(520.))),
                    ..Default::default()
                },
                |window, cx| {
                    let gallery = cx.new(|cx| {
                        let mut gallery = Gallery::new(items, outputs, initial, cx);
                        gallery.focus.focus(window);
                        gallery.refresh_safety(cx);
                        gallery
                    });
                    cx.new(|cx| Root::new(gallery, window, cx))
                },
            )
            .expect("open gallery window");
            cx.activate(true);
        });
    Ok(())
}
#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's glob export includes a `test` macro that would
    // shadow the built-in attribute `#[gpui::test]` expands to.
    use super::{Gallery, PathBuf, Update, Verification, library, safety, ui};
    use gpui::TestAppContext;
    use std::{cell::Cell, rc::Rc};
    fn wallpaper(id: &str) -> library::Wallpaper {
        library::Wallpaper {
            id: id.into(),
            directory: PathBuf::from("/fixture").join(id),
            title: format!("Fixture {id}"),
            kind: "scene".into(),
            tags: Vec::new(),
            description: String::new(),
            properties: serde_json::Value::Null,
            preview: None,
        }
    }
    fn safe() -> safety::Status {
        safety::Status {
            label: "Safe",
            allowed: true,
            reason: "synthetic".into(),
            review: None,
        }
    }
    // Results produced on another thread while the window is idle must reach
    // the root view as a notify that invalidates the window and re-renders it.
    #[gpui::test]
    fn background_verification_rerenders_root_view(cx: &mut TestAppContext) {
        let items = vec![wallpaper("1"), wallpaper("2"), wallpaper("3")];
        cx.update(ui::init);
        let window = cx.add_window(|_, cx| Gallery::new(items, Vec::new(), Some(0), cx));
        let gallery = window.root(cx).unwrap();
        let notifications = Rc::new(Cell::new(0));
        cx.update(|cx| {
            let notifications = notifications.clone();
            cx.observe(&gallery, move |_, _| {
                notifications.set(notifications.get() + 1)
            })
            .detach()
        });
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        window
            .update(cx, |gallery, _, cx| gallery.listen(receiver, None, cx))
            .unwrap();
        // The selection appears first; later results must still repaint even
        // though they no longer change the selection.
        sender.unbounded_send(Update::Item(0, safe())).unwrap();
        cx.run_until_parked();
        let before = notifications.get();
        let renders = gallery.read_with(cx, |gallery, _| gallery.renders);
        std::thread::spawn(move || {
            sender
                .unbounded_send(Update::Item(1, safety::Status::unreviewed("no review")))
                .unwrap();
            sender.unbounded_send(Update::Item(2, safe())).unwrap();
            sender.unbounded_send(Update::Complete).unwrap();
        })
        .join()
        .unwrap();
        cx.run_until_parked();
        assert!(
            notifications.get() > before,
            "completion did not notify the root view"
        );
        gallery.read_with(cx, |gallery, _| {
            // The test platform draws dirty windows when effects flush.
            assert!(gallery.renders > renders, "window was not invalidated");
            assert_eq!(gallery.verification, Verification::Ready);
            assert_eq!(gallery.visible, vec![0, 2]);
            assert_eq!(gallery.selected, 0);
        });
    }
    #[gpui::test]
    fn missing_selection_falls_back_only_after_verification(cx: &mut TestAppContext) {
        let items = vec![wallpaper("1"), wallpaper("2")];
        cx.update(ui::init);
        let window = cx.add_window(|_, cx| Gallery::new(items, Vec::new(), Some(1), cx));
        let gallery = window.root(cx).unwrap();
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        window
            .update(cx, |gallery, _, cx| gallery.listen(receiver, None, cx))
            .unwrap();
        sender.unbounded_send(Update::Item(0, safe())).unwrap();
        cx.run_until_parked();
        gallery.read_with(cx, |gallery, _| assert_eq!(gallery.selected, 1));
        sender
            .unbounded_send(Update::Item(1, safety::Status::unreviewed("changed")))
            .unwrap();
        sender.unbounded_send(Update::Complete).unwrap();
        cx.run_until_parked();
        gallery.read_with(cx, |gallery, _| assert_eq!(gallery.selected, 0));
    }
}
