mod backend;
mod library;
mod preview;
use backend::ApplyBackend;
use gpui::{prelude::*, *};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};
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
}
actions!(gallery, [Quit, Apply, Next, Previous, NextOutput]);
impl Gallery {
    fn new(
        items: Vec<library::Wallpaper>,
        outputs: Vec<backend::Output>,
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
        }
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
            self.large_task = Some(cx.spawn(async move |this, cx| {
                if let Ok(result) = receiver.await {
                    _ = this.update(cx, |this, cx| {
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
            cx.spawn(async move |this, cx| {
                if let Ok(result) = receiver.await {
                    _ = this.update(cx, |this, cx| {
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
        cx.spawn(async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                this.busy = false;
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
        let columns =
            (((f32::from(window.bounds().size.width) - 388.) / 190.).floor() as usize).clamp(1, 12);
        self.cache_limit =
            (columns * ((f32::from(window.bounds().size.height) / 164.).ceil() as usize + 3) + 64)
                .max(128);
        if columns != self.last_columns {
            self.scroll_selection = true;
            self.last_columns = columns;
        }
        if self.scroll_selection {
            self.scroll
                .scroll_to_item(self.selected / columns, ScrollStrategy::Center);
            self.scroll_selection = false;
        }
        let rows = self.items.len().div_ceil(columns);
        let tile_width =
            ((f32::from(window.bounds().size.width) - 372. - 12. * (columns - 1) as f32)
                / columns as f32)
                .max(80.);
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
        if let Some(item) = self.items.get(self.selected).cloned() {
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
                    .when(!self.busy && !self.outputs.is_empty(), |s| {
                        s.on_click(cx.listener(|this, _, _, cx| this.apply(cx)))
                    })
                    .when(self.busy || self.outputs.is_empty(), |s| s.opacity(0.5)),
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
        div().track_focus(&self.focus).key_context("Gallery").size_full().flex().flex_col().bg(rgb(0x181b20))

            .font_family("sans-serif").text_sm().text_color(rgb(0xedf0f4))
            .on_action(|_: &Quit,_,cx|cx.quit())
            .on_action(cx.listener(|this,_:&Apply,_,cx|this.apply(cx)))
            .on_action(cx.listener(|this,_:&Next,_,cx|if !this.items.is_empty(){this.select((this.selected+1).min(this.items.len()-1),cx)}))
            .on_action(cx.listener(|this,_:&Previous,_,cx|if !this.items.is_empty(){this.select(this.selected.saturating_sub(1),cx)}))
            .on_action(cx.listener(|this,_:&NextOutput,_,cx|if !this.outputs.is_empty(){this.output=(this.output+1)%this.outputs.len();cx.notify();}))
            .child(top)
            .child(div().flex().flex_1().min_h_0()
                .child(div().flex_1().min_w_0().h_full().flex().flex_col().p_4().gap_3()
                    .child(div().flex().justify_between().child(format!("Installed wallpapers  ·  {}",self.items.len())).child(div().text_color(rgb(0xb6beca)).child("← → Select   ·   Tab Display   ·   Enter Apply")))
                    .when(self.items.is_empty(),|s|s.child("No wallpapers found. Install Wallpaper Engine Workshop items through Steam, then reopen WPE Gallery."))
                    .child(uniform_list("wallpapers",rows,cx.processor(move |this,range: std::ops::Range<usize>,_,cx| {
                        range.map(|row| {
                            let mut line = div().flex().gap_3().h(px(164.)).pb_3();
                            for index in row*columns..((row+1)*columns).min(this.items.len()) {
                                let source = this.thumb(index,cx);
                                let item = &this.items[index];
                                let mut tile = div().id(("tile",index)).w(px(tile_width)).flex_shrink_0().min_w_0().h_full().flex().flex_col()
                                    .bg(rgb(0x22262d)).border_2().border_color(if index == this.selected {rgb(0x569dff)}else{rgb(0x303741)})
                                    .cursor_pointer().hover(|s|s.bg(rgb(0x303741)))
                                    .on_click(cx.listener(move |this,_,_,cx|this.select(index,cx)));
                                let mut artwork = div().w_full().h(px(112.)).bg(rgb(0x121519)).overflow_hidden().flex().items_center().justify_center();
                                if let Some(source) = source { artwork = artwork.child(img(source).size_full().object_fit(ObjectFit::Cover)); }
                                else { artwork = artwork.child("No preview"); }
                                tile = tile.child(artwork).child(div().px_2().py_1().truncate().child(item.title.clone()));
                                line = line.child(tile);
                            }
                            line
                        }).collect()
                    })).track_scroll(self.scroll.clone()).flex_1().min_h_0()))
                .child(details))
            .child(div().px_4().py_2().min_h(px(34.)).bg(rgb(0x22262d)).child(self.status.clone()))
    }
}
fn main() -> anyhow::Result<()> {
    let (items, warnings) = library::scan();
    let outputs = backend::outputs();
    let args: Vec<_> = std::env::args().collect();
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
                    let mut gallery = Gallery::new(items, outputs, cx);
                    if let Some(index) = initial {
                        gallery.selected = index;
                    }
                    gallery.focus.focus(window);
                    if !gallery.items.is_empty() {
                        gallery.select(gallery.selected, cx);
                    }
                    gallery
                })
            },
        )
        .expect("open gallery window");
        cx.activate(true);
    });
    Ok(())
}
