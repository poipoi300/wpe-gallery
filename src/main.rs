use wpe_gallery::{backend, library, renderer, safety};
mod grid;
mod preview;
mod verification;
use anyhow::Context as _;
use backend::ApplyBackend;
use gpui::{prelude::*, *};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
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
    thumbnails: HashMap<usize, Option<Arc<RenderImage>>>,
    pending: HashSet<usize>,
    lru: VecDeque<usize>,
    large: Option<Arc<RenderImage>>,
    generation: usize,
    large_task: Option<Task<()>>,
    cache_limit: usize,
    scroll: UniformListScrollHandle,
    scroll_selection: bool,
    last_columns: usize,
    focus: FocusHandle,
    safety: Vec<safety::Status>,
    visible: Vec<usize>,
    safe_only: bool,
    safety_task: Option<Task<()>>,
    verification: Verification,
    window: AnyWindowHandle,
}
actions!(gallery, [Quit, Apply, Next, Previous, NextOutput]);
impl Gallery {
    fn new(
        items: Vec<library::Wallpaper>,
        outputs: Vec<backend::Output>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected = library::load_config(&library::config_path())
            .ok()
            .and_then(|c| {
                outputs
                    .first()
                    .and_then(|o| c.outputs.get(&o.name))
                    .map(|a| a.wallpaper_id.clone())
            })
            .and_then(|id| items.iter().position(|i| i.id == id))
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
            pending: HashSet::new(),
            lru: VecDeque::new(),
            large: None,
            generation: 0,
            large_task: None,
            cache_limit: 128,
            scroll: UniformListScrollHandle::new(),
            scroll_selection: true,
            last_columns: 0,
            focus: cx.focus_handle(),
            safety: vec![safety::Status::unreviewed("Checking reviews…"); count],
            visible: Vec::new(),
            safe_only: true,
            safety_task: None,
            verification: Verification::Checking {
                completed: 0,
                total: count,
            },
            window: window.window_handle(),
        }
    }
    fn update_in_window(
        view: &WeakEntity<Self>,
        window: AnyWindowHandle,
        cx: &mut AsyncApp,
        update: impl FnOnce(&mut Self, &mut Context<Self>),
    ) -> anyhow::Result<()> {
        window.update(cx, |_, window, cx| {
            let result = view.update(cx, |this, cx| {
                update(this, cx);
                cx.notify();
            });
            window.refresh();
            result
        })?
    }
    fn rebuild_visible(&mut self, cx: &mut Context<Self>) {
        self.visible = (0..self.items.len())
            .filter(|i| !self.safe_only || self.safety[*i].allowed)
            .collect();
        if !self.visible.contains(&self.selected) {
            self.large_task.take();
            if let Some(image) = self.large.take() {
                cx.drop_image(image, None);
            }
            if let Some(index) = self.visible.first().copied() {
                self.select(index, cx);
            }
        }
        if self.large.is_none()
            && self.large_task.is_none()
            && self.visible.contains(&self.selected)
        {
            self.select(self.selected, cx);
        }
        self.scroll_selection = true;
        cx.notify();
    }
    fn refresh_safety(&mut self, cx: &mut Context<Self>) {
        self.safety_task.take();
        self.verification = Verification::Checking {
            completed: 0,
            total: self.items.len(),
        };
        self.safety = vec![safety::Status::unreviewed("Checking reviews…"); self.items.len()];
        self.rebuild_visible(cx);
        let items = self.items.clone();
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        enum Update {
            Item(usize, safety::Status),
            Complete,
            Failed(String),
        }
        let worker = cx.background_executor().spawn(async move {
            let reviews = match safety::read_whitelist(&safety::whitelist_path()) {
                Ok(reviews) => reviews,
                Err(error) => {
                    let _ = sender.unbounded_send(Update::Failed(format!(
                        "Could not load whitelist {}: {error:#}",
                        safety::whitelist_path().display()
                    )));
                    return;
                }
            };
            for (index, item) in items.iter().enumerate() {
                if sender.is_closed() {
                    return;
                }
                let status = safety::assess(&reviews, &item.id, &item.directory);
                if sender.unbounded_send(Update::Item(index, status)).is_err() {
                    return;
                }
            }
            let _ = sender.unbounded_send(Update::Complete);
        });
        let window = self.window;
        self.safety_task = Some(cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            let _worker = worker;
            while let Some(update) = receiver.next().await {
                if Self::update_in_window(&this, window, cx, |this, cx| {
                    match update {
                        Update::Item(index, status) => {
                            this.safety[index] = status;
                            if let Verification::Checking { completed, .. } = &mut this.verification
                            {
                                *completed += 1;
                            }
                        }
                        Update::Complete => {
                            this.verification = Verification::Ready;
                            eprintln!(
                                "Whitelist verification complete: {} of {} wallpapers safe",
                                this.safety.iter().filter(|s| s.allowed).count(),
                                this.items.len()
                            );
                        }
                        Update::Failed(error) => {
                            eprintln!("{error}");
                            this.verification = Verification::Failed(error);
                        }
                    }
                    this.rebuild_visible(cx);
                })
                .is_err()
                {
                    break;
                }
            }
        }));
    }
    fn move_selection(&mut self, next: bool, cx: &mut Context<Self>) {
        if self.visible.is_empty() {
            return;
        }
        let position = self
            .visible
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0);
        let position = if next {
            (position + 1).min(self.visible.len() - 1)
        } else {
            position.saturating_sub(1)
        };
        self.select(self.visible[position], cx);
    }
    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.large_task.take();
        self.selected = index;
        self.scroll_selection = true;
        self.generation += 1;
        if let Some(image) = self.large.take() {
            cx.drop_image(image, None);
        }
        let generation = self.generation;
        if let Some(path) = self.items.get(index).and_then(|i| i.preview.clone())
            && let Some(receiver) = self.pool.request(path, true)
        {
            let window = self.window;
            self.large_task = Some(cx.spawn(async move |this, cx| {
                if let Ok(result) = receiver.await {
                    _ = Self::update_in_window(&this, window, cx, |this, cx| {
                        if this.generation == generation {
                            match result {
                                Ok(image) => this.large = Some(image),
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
    fn thumb(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        if let Some(value) = self.thumbnails.get(&index) {
            self.lru.retain(|i| *i != index);
            self.lru.push_back(index);
            return value.clone();
        }
        if !self.pending.contains(&index)
            && let Some(path) = self.items[index].preview.clone()
            && let Some(receiver) = self.pool.request(path, false)
        {
            self.pending.insert(index);
            let window = self.window;
            cx.spawn(async move |this, cx| {
                if let Ok(result) = receiver.await {
                    _ = Self::update_in_window(&this, window, cx, |this, cx| {
                        this.pending.remove(&index);
                        this.thumbnails.insert(index, result.ok());
                        this.lru.push_back(index);
                        while this.lru.len() > this.cache_limit {
                            if let Some(old) = this.lru.pop_front()
                                && let Some(Some(image)) = this.thumbnails.remove(&old)
                            {
                                cx.drop_image(image, None);
                            }
                        }
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        None
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
        let (output, id) = (output.name.clone(), item.id.clone());
        self.busy = true;
        self.status = "Applying wallpaper…".into();
        cx.notify();
        let job = cx.background_executor().spawn(async move {
            backend::Platform
                .apply(&output, &id)
                .map(|_| output)
                .map_err(|e| e.to_string())
        });
        let window = self.window;
        cx.spawn(async move |this, cx| {
            let result = job.await;
            _ = Self::update_in_window(&this, window, cx, |this, cx| {
                this.busy = false;
                if result.is_err() {
                    this.refresh_safety(cx);
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
fn button(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_sm()
        .bg(rgb(0x303741))
        .text_color(rgb(0xedf0f4))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x414c5b)))
        .child(text.into())
}
impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let grid = grid::GridLayout::new(viewport.width.into(), viewport.height.into());
        let columns = grid.columns;
        self.cache_limit = grid.cache_limit;
        if columns != self.last_columns {
            self.scroll_selection = true;
            self.last_columns = columns;
        }
        if self.scroll_selection {
            self.scroll.scroll_to_item(
                self.visible
                    .iter()
                    .position(|i| *i == self.selected)
                    .unwrap_or(0)
                    / columns,
                ScrollStrategy::Center,
            );
            self.scroll_selection = false;
        }
        let rows = self.visible.len().div_ceil(columns);
        let tile_width = grid.tile_width;
        let mut top = div()
            .flex()
            .items_center()
            .gap_3()
            .p_4()
            .bg(rgb(0x22262d))
            .border_b_1()
            .border_color(rgb(0x363d47))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .mr_4()
                    .child("WPE Gallery"),
            );
        for (i, o) in self.outputs.iter().enumerate() {
            top = top.child(
                button(
                    ("output", i),
                    format!("{}   {} × {}", o.name, o.width, o.height),
                )
                .when(i == self.output, |s| s.bg(rgb(0x325c8a)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.output = i;
                    cx.notify();
                })),
            );
        }
        if self.outputs.is_empty() {
            top = top.child("No connected outputs");
        }
        let mut details = div()
            .id("details")
            .w(px(340.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .p_5()
            .bg(rgb(0x22262d))
            .flex()
            .flex_col()
            .gap_4();
        if let Some(item) = self
            .items
            .get(self.selected)
            .filter(|_| self.visible.contains(&self.selected))
            .cloned()
        {
            let mut preview = div()
                .w_full()
                .h(px(210.))
                .bg(rgb(0x121519))
                .flex()
                .justify_center()
                .items_center();
            let source = self
                .large
                .clone()
                .or_else(|| self.thumbnails.get(&self.selected).cloned().flatten());
            if let Some(source) = source {
                preview = preview.child(
                    img(source)
                        .id(("selected", self.generation))
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                );
            } else {
                preview = preview.child("Preview unavailable");
            }
            details = details
                .child(preview)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.safety[self.selected].label),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0xb6beca))
                        .child(self.safety[self.selected].reason.clone()),
                )
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(item.title),
                )
                .child(div().text_color(rgb(0xb6beca)).child(format!(
                    "{}  ·  Workshop {}",
                    item.kind.to_uppercase(),
                    item.id
                )))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .children(item.tags.into_iter().map(|t| {
                            div()
                                .px_2()
                                .py_1()
                                .bg(rgb(0x303741))
                                .rounded_sm()
                                .text_sm()
                                .child(t)
                        })),
                )
                .child(
                    button(
                        "apply",
                        if self.busy {
                            "Applying…"
                        } else {
                            "Apply wallpaper"
                        },
                    )
                    .bg(rgb(0x2866ad))
                    .when(
                        !self.busy
                            && !self.outputs.is_empty()
                            && self.safety[self.selected].allowed,
                        |s| s.on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    )
                    .when(
                        self.busy || self.outputs.is_empty() || !self.safety[self.selected].allowed,
                        |s| s.opacity(0.5),
                    ),
                )
                .child(div().text_sm().text_color(rgb(0xb6beca)).child(
                    if item.description.is_empty() {
                        "No description provided.".to_string()
                    } else {
                        item.description
                    },
                ))
                .child(div().text_sm().text_color(rgb(0xb6beca)).child(format!(
                    "{} configurable properties · editor coming later",
                    item.properties.as_object().map_or(0, |p| p.len())
                )));
        } else {
            details = details.child("Select a wallpaper to see its details.");
        }
        div()
            .track_focus(&self.focus)
            .key_context("Gallery")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x181b20))
            .font_family("sans-serif")
            .text_sm()
            .text_color(rgb(0xedf0f4))
            .on_action(|_: &Quit, _, cx| cx.quit())
            .on_action(cx.listener(|this, _: &Apply, _, cx| this.apply(cx)))
            .on_action(cx.listener(|this, _: &Next, _, cx| this.move_selection(true, cx)))
            .on_action(cx.listener(|this, _: &Previous, _, cx| this.move_selection(false, cx)))
            .on_action(cx.listener(|this, _: &NextOutput, _, cx| {
                if !this.outputs.is_empty() {
                    this.output = (this.output + 1) % this.outputs.len();
                    cx.notify();
                }
            }))
            .child(top)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .child(
                        button(
                            "safe-filter",
                            if self.safe_only {
                                "Safe only: on"
                            } else {
                                "Safe only: off"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.safe_only = !this.safe_only;
                            this.rebuild_visible(cx);
                        })),
                    )
                    .child(
                        button("refresh-reviews", "Reload reviews")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_safety(cx))),
                    )
                    .child("Only reviewed, unchanged items can be applied."),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .p_4()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .child(format!(
                                        "Installed wallpapers  ·  {} of {}",
                                        self.visible.len(),
                                        self.items.len()
                                    ))
                                    .child(
                                        div().text_color(rgb(0xb6beca)).child(
                                            "← → Select   ·   Tab Display   ·   Enter Apply",
                                        ),
                                    ),
                            )
                            .when(self.visible.is_empty(), |s| {
                                s.child(self.verification.empty_message(!self.items.is_empty()))
                            })
                            .when(!self.visible.is_empty(), |s| {
                                s.when_some(self.verification.progress(), |s, message| {
                                    s.child(message)
                                })
                                .when(matches!(self.verification, Verification::Failed(_)), |s| {
                                    s.child(self.verification.empty_message(true))
                                })
                            })
                            .child(
                                uniform_list(
                                    "wallpapers",
                                    rows,
                                    cx.processor(
                                        move |this, range: std::ops::Range<usize>, _, cx| {
                                            range
                                                .map(|row| {
                                                    let mut line =
                                                        div().flex().gap_3().h(px(164.)).pb_3();
                                                    for position in row * columns
                                                        ..((row + 1) * columns)
                                                            .min(this.visible.len())
                                                    {
                                                        let index = this.visible[position];
                                                        let source = this.thumb(index, cx);
                                                        let item = &this.items[index];
                                                        let mut tile = div()
                                                            .id(("tile", index))
                                                            .w(px(tile_width))
                                                            .flex_shrink_0()
                                                            .min_w_0()
                                                            .h_full()
                                                            .flex()
                                                            .flex_col()
                                                            .bg(rgb(0x22262d))
                                                            .border_2()
                                                            .border_color(
                                                                if index == this.selected {
                                                                    rgb(0x569dff)
                                                                } else {
                                                                    rgb(0x303741)
                                                                },
                                                            )
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(rgb(0x303741)))
                                                            .on_mouse_down(
                                                                MouseButton::Left,
                                                                cx.listener(
                                                                    move |this, _, _, cx| {
                                                                        this.select(index, cx);
                                                                        cx.stop_propagation();
                                                                    },
                                                                ),
                                                            );
                                                        let mut artwork = div()
                                                            .w_full()
                                                            .h(px(92.))
                                                            .bg(rgb(0x121519))
                                                            .overflow_hidden()
                                                            .flex()
                                                            .items_center()
                                                            .justify_center();
                                                        if let Some(source) = source {
                                                            artwork = artwork.child(
                                                                img(source)
                                                                    .size_full()
                                                                    .object_fit(ObjectFit::Cover),
                                                            );
                                                        } else {
                                                            artwork = artwork.child("No preview");
                                                        }
                                                        tile = tile
                                                            .child(artwork)
                                                            .child(
                                                                div()
                                                                    .px_2()
                                                                    .text_xs()
                                                                    .text_color(rgb(0xb6beca))
                                                                    .child(
                                                                        this.safety[index].label,
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .px_2()
                                                                    .py_1()
                                                                    .truncate()
                                                                    .child(item.title.clone()),
                                                            );
                                                        line = line.child(tile);
                                                    }
                                                    line
                                                })
                                                .collect()
                                        },
                                    ),
                                )
                                .with_sizing_behavior(ListSizingBehavior::Auto)
                                .w_full()
                                .track_scroll(self.scroll.clone())
                                .flex_1()
                                .min_h_0(),
                            ),
                    )
                    .child(details),
            )
            .child(
                div()
                    .px_4()
                    .py_2()
                    .min_h(px(34.))
                    .bg(rgb(0x22262d))
                    .child(self.status.clone()),
            )
    }
}
fn main() -> anyhow::Result<()> {
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
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("ctrl-q", Quit, None),
            KeyBinding::new("enter", Apply, Some("Gallery")),
            KeyBinding::new("right", Next, Some("Gallery")),
            KeyBinding::new("left", Previous, Some("Gallery")),
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
                cx.new(|cx| {
                    let mut gallery = Gallery::new(items, outputs, window, cx);
                    if let Some(index) = initial {
                        gallery.selected = index;
                    }
                    cx.observe_window_bounds(window, |this, window, cx| {
                        this.scroll_selection = true;
                        cx.notify();
                        window.refresh();
                    })
                    .detach();
                    gallery.focus.focus(window);
                    gallery.refresh_safety(cx);
                    gallery
                })
            },
        )
        .expect("open gallery window");
        cx.activate(true);
    });
    Ok(())
}
