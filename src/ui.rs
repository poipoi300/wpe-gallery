use super::*;
use gpui::prelude::*;
use gpui_component::{
    Disableable, Icon, IconName, Sizable, StyledExt, Theme, ThemeMode, box_shadow,
    button::{Button, ButtonVariants},
    collapsible::Collapsible,
    divider::Divider,
    h_flex,
    kbd::Kbd,
    label::Label,
    list::ListItem,
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
    tag::Tag,
    v_flex, v_virtual_list,
};
use std::rc::Rc;

const INK: u32 = 0xf2f1ed;
const MUTED: u32 = 0xaaaead;
const EDGE: u32 = 0x373937;
const PEARL: u32 = 0xd8e8da;

pub(crate) fn init(cx: &mut App) {
    gpui_component::init(cx);
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_family = "Inter".into();
    theme.radius = px(7.);
    theme.background = rgb(0x141515).into();
    theme.foreground = rgb(INK).into();
    theme.muted_foreground = rgb(MUTED).into();
    theme.border = rgb(EDGE).into();
    theme.primary = rgb(PEARL).into();
    theme.primary_hover = rgb(0xe8f1e9).into();
    theme.primary_active = rgb(0xbacdbd).into();
    theme.primary_foreground = rgb(0x19251d).into();
    theme.secondary = rgb(0x292b2a).into();
    theme.secondary_hover = rgb(0x333633).into();
    theme.list_hover = rgb(0x373a38).into();
    theme.list_active = rgb(0x252c28).into();
    theme.list_active_border = rgb(0xb1ccb6).into();
    theme.ring = rgb(PEARL).into();
}

fn muted(text: impl Into<SharedString>) -> Label {
    Label::new(text).text_sm().text_color(rgb(MUTED))
}

fn section(text: &'static str) -> Label {
    Label::new(text)
        .text_size(px(11.))
        .font_semibold()
        .text_color(rgb(MUTED))
}

impl Gallery {
    fn output_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let output = self.outputs.get(self.output);
        let gallery = cx.entity().downgrade();
        let caption = output
            .map(|o| format!("{}   ·   {} × {}", o.name, o.width, o.height))
            .unwrap_or_else(|| "No connected outputs".into());
        Button::new("output")
            .label(caption)
            .icon(IconName::PanelBottom)
            .dropdown_caret(self.outputs.len() > 1)
            .disabled(output.is_none())
            .dropdown_menu_with_anchor(Corner::TopRight, move |mut menu, _, cx| {
                if let Some(gallery) = gallery.upgrade() {
                    let state = gallery.read(cx);
                    for (i, output) in state.outputs.iter().enumerate() {
                        let gallery = gallery.downgrade();
                        menu = menu.item(
                            PopupMenuItem::new(format!(
                                "{}   ·   {} × {}",
                                output.name, output.width, output.height
                            ))
                            .checked(i == state.output)
                            .on_click(move |_, window, cx| {
                                _ = gallery.update(cx, |this, cx| {
                                    this.output = i;
                                    this.focus.focus(window);
                                    cx.notify();
                                });
                            }),
                        );
                    }
                }
                menu
            })
    }

    fn tile(
        &mut self,
        index: usize,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ListItem {
        let source = self.thumb(index, cx);
        let light = self
            .thumbnails
            .get(&index)
            .map(|t| t.light)
            .unwrap_or_default();
        let selected = index == self.selected;
        let status = &self.safety[index];
        let hovered = self
            .hovered
            .filter(|(i, _)| *i == index)
            .map(|(_, since)| since);
        let caption = title::caption(
            &self.items[index].title,
            width - 26. - if selected { 24. } else { 0. },
            hovered,
            &self.marquee_wake,
            window,
            cx,
        );
        let art = v_flex()
            .relative()
            .items_center()
            .justify_center()
            .w_full()
            .h(px((width * 9. / 16.).round()))
            .bg(rgb(0x111212))
            .overflow_hidden()
            .map(|art| match source {
                Some(source) => art.child(
                    img(source)
                        .size_full()
                        .rounded_t(px(8.))
                        .object_fit(ObjectFit::Cover),
                ),
                None => art.child(Icon::new(IconName::GalleryVerticalEnd).text_color(rgb(MUTED))),
            })
            .child(
                v_flex()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(25.))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(rgba(0x15171600), 0.),
                        linear_color_stop(rgba(0x15171690), 1.),
                    )),
            )
            .when(!status.allowed, |s| {
                s.child(
                    h_flex().absolute().top_2().left_2().child(
                        Tag::custom(
                            rgb(0x322a24).into(),
                            rgb(0xf0d5bd).into(),
                            rgb(0x665647).into(),
                        )
                        .small()
                        .child(status.label),
                    ),
                )
            });
        ListItem::new(("tile", index))
            .selected(selected)
            .w(px(width))
            .flex_shrink_0()
            .p_0()
            .rounded(px(9.))
            .overflow_hidden()
            .bg(rgb(0x202221))
            .border_1()
            .border_color(if selected { rgb(0xb1ccb6) } else { rgb(EDGE) })
            .shadow(vec![box_shadow(0., 3., 10., -3., light.halo())])
            .child(
                v_flex()
                    .id(("artwork", index))
                    .w(px(width - 2.))
                    .min_w_0()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.focus.focus(window);
                            this.choose(index, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .on_hover(cx.listener(|_, _, _, cx| cx.notify()))
                    .child(art)
                    .child(
                        h_flex()
                            .h(px(40.))
                            .px(px(12.))
                            .rounded_b(px(8.))
                            .gap_2()
                            .bg(light.wash(0.28))
                            .child(caption)
                            .when(selected, |s| {
                                s.child(
                                    Icon::new(IconName::CircleCheck)
                                        .small()
                                        .flex_shrink_0()
                                        .text_color(rgb(PEARL)),
                                )
                            }),
                    ),
            )
    }

    fn review(&self, cx: &mut Context<Self>) -> AnyElement {
        let status = &self.safety[self.selected];
        if !status.allowed {
            return v_flex()
                .gap_1()
                .child(Label::new(format!("{} · Apply unavailable", status.label)).font_medium())
                .child(muted(status.reason.clone()))
                .into_any_element();
        }
        let Some(review) = &status.review else {
            return muted("Marked safe").into_any_element();
        };
        Collapsible::new()
            .open(self.review_open)
            .child(
                Button::new("review")
                    .ghost()
                    .compact()
                    .small()
                    .icon(if self.review_open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .label("Review details")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.review_open = !this.review_open;
                        cx.notify();
                    })),
            )
            .content(
                v_flex()
                    .gap_1()
                    .pt_2()
                    .child(muted(format!("{} · {}", review.reviewer, review.reviewed)))
                    .child(
                        muted(if review.notes.trim().is_empty() {
                            "No notes".to_owned()
                        } else {
                            review.notes.clone()
                        })
                        .text_sm(),
                    )
                    .child(section("SHA-256"))
                    .child(
                        muted(review.content_hash.clone())
                            .font_family("monospace")
                            .text_xs(),
                    ),
            )
            .into_any_element()
    }

    fn details(&self, cx: &mut Context<Self>) -> Div {
        let mut pane = v_flex()
            .relative()
            .h_full()
            .w(px(grid::INSPECTOR))
            .flex_shrink_0()
            .bg(rgba(0x2426269c))
            .border_l_1()
            .border_color(rgb(EDGE));
        let header = h_flex()
            .h(px(53.))
            .px_5()
            .flex_shrink_0()
            .gap_2()
            .child(
                Icon::new(IconName::PanelRight)
                    .small()
                    .text_color(rgb(MUTED)),
            )
            .child(Label::new("Inspector").font_medium());
        let Some(item) = self
            .items
            .get(self.selected)
            .filter(|_| self.visible.contains(&self.selected))
        else {
            return pane.child(header).child(
                v_flex()
                    .p_5()
                    .child(muted("Select a wallpaper to inspect it.")),
            );
        };
        let thumb = self.thumbnails.get(&self.selected);
        let light = thumb.map(|t| t.light).unwrap_or_default();
        let image = self
            .large
            .get(self.large_frame)
            .map(|(i, _)| i.clone())
            .or_else(|| thumb.and_then(|t| t.image.clone()));
        let allowed = self.safety[self.selected].allowed;
        let preview_height = image.as_ref().map_or(180., |image| {
            let size = image.size(0);
            ((grid::INSPECTOR - 42.) * size.height.0 as f32 / size.width.0.max(1) as f32)
                .clamp(150., 300.)
        });
        let art = v_flex()
            .w_full()
            .h(px(preview_height))
            .rounded(px(9.))
            .overflow_hidden()
            .items_center()
            .justify_center()
            .bg(rgb(0x121413))
            .border_1()
            .border_color(rgba(0xffffff22))
            .shadow(vec![box_shadow(0., 6., 24., 0., light.halo())])
            .child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .bg(light.wash(0.36))
                    .map(|s| match image {
                        Some(i) => s.child(
                            img(i)
                                .id(("selected", self.generation))
                                .rounded(px(8.))
                                .size_full()
                                .object_fit(ObjectFit::Contain),
                        ),
                        None => s.child(muted("Preview unavailable")),
                    }),
            );
        let tags = h_flex().flex_wrap().gap_1p5().children(
            item.tags
                .iter()
                .map(|t| Tag::secondary().outline().small().child(t.clone())),
        );
        let fields = v_flex()
            .gap_2()
            .child(detail_row("Type", friendly_kind(&item.kind)))
            .child(detail_row("Workshop ID", item.id.clone()))
            .child(detail_row(
                "Properties",
                item.properties
                    .as_object()
                    .map_or(0, |p| p.len())
                    .to_string(),
            ));
        let body = v_flex()
            .id("inspector-body")
            .min_h_0()
            .overflow_y_scroll()
            .flex_1()
            .child(
                v_flex().gap_4().px_5().pb_4().pt_1().child(art).child(
                    v_flex()
                        .gap_2()
                        .child(
                            Label::new(item.title.clone())
                                .text_size(px(25.))
                                .line_height(px(31.))
                                .font_semibold(),
                        )
                        .when(!item.tags.is_empty(), |s| s.child(tags)),
                ),
            )
            .child(
                v_flex()
                    .px_5()
                    .pb_5()
                    .gap_4()
                    .child(Divider::horizontal())
                    .child(fields)
                    .when(!item.description.trim().is_empty(), |s| {
                        s.child(
                            v_flex()
                                .gap_1()
                                .child(section("DESCRIPTION"))
                                .child(muted(item.description.clone())),
                        )
                    })
                    .child(Divider::horizontal())
                    .child(
                        v_flex()
                            .items_start()
                            .gap_1()
                            .child(section("VERIFICATION"))
                            .when(allowed, |s| {
                                s.child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Icon::new(IconName::CircleCheck)
                                                .small()
                                                .text_color(rgb(0xaacdb5)),
                                        )
                                        .child(Label::new("Verified safe")),
                                )
                            })
                            .child(self.review(cx)),
                    ),
            );
        let footer = v_flex()
            .flex_shrink_0()
            .px_5()
            .py_4()
            .gap_2()
            .border_t_1()
            .border_color(rgb(EDGE))
            .when(!self.status.is_empty(), |s| {
                s.child(muted(self.status.clone()))
            })
            .child(
                Button::new("apply")
                    .primary()
                    .large()
                    .w_full()
                    .icon(IconName::Check)
                    .label(if self.busy {
                        "Applying…"
                    } else {
                        "Apply wallpaper"
                    })
                    .disabled(self.busy || !allowed || self.outputs.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
            );
        pane = pane.child(
            v_flex()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(px(316.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(light.halo(), 0.),
                    linear_color_stop(rgba(0), 1.),
                )),
        );
        pane.child(header).child(body).child(footer)
    }
}

fn detail_row(name: &'static str, value: String) -> Div {
    h_flex()
        .justify_between()
        .gap_2()
        .child(muted(name))
        .child(Label::new(value).text_sm())
}

fn friendly_kind(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Unknown".into(),
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        self.frame += 1;
        self.cancel_offscreen_thumbnails();
        let viewport = window.viewport_size();
        let grid = grid::GridLayout::new(viewport.width.into(), viewport.height.into());
        let columns = grid.columns;
        self.cache_limit = grid.cache_limit;
        if columns != self.last_columns {
            self.scroll_selection = true;
            self.last_columns = columns;
        }
        if self.scroll_selection {
            if let Some(position) = self.visible.iter().position(|i| *i == self.selected) {
                self.scroll
                    .scroll_to_item(position / columns, ScrollStrategy::Center);
            }
            self.scroll_selection = false;
        }
        let rows = self.visible.len().div_ceil(columns);
        let tile_width = grid.tile_width;
        let row_height = grid.row_height;
        let row_size = size(px(0.), px(row_height));
        if self.row_sizes.len() != rows || self.row_sizes.first().is_some_and(|s| *s != row_size) {
            self.row_sizes = Rc::new(vec![row_size; rows]);
        }
        let list = v_virtual_list(
            cx.entity(),
            "wallpapers",
            self.row_sizes.clone(),
            move |this, range, window, cx| {
                let viewport = this.scroll.bounds();
                let pointer = window.mouse_position();
                let local = pointer - viewport.origin - this.scroll.offset();
                let hovered = (window.is_window_hovered() && viewport.contains(&pointer))
                    .then(|| grid.position_at(local.x.into(), local.y.into()))
                    .flatten()
                    .and_then(|pos| this.visible.get(pos).copied());
                if this.hovered.map(|(i, _)| i) != hovered {
                    this.hovered = hovered.map(|i| (i, Instant::now()));
                    this.marquee_wake.borrow_mut().take();
                }

                let elements =
                    range
                        .clone()
                        .map(|row| {
                            let end = ((row + 1) * columns).min(this.visible.len());
                            h_flex()
                                .items_start()
                                .gap(px(grid::GAP))
                                .h(px(row_height))
                                .children((row * columns..end).map(|pos| {
                                    this.tile(this.visible[pos], tile_width, window, cx)
                                }))
                        })
                        .collect();
                this.prefetch(range, columns, cx);
                elements
            },
        )
        .track_scroll(&self.scroll)
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .w_full()
        .flex_1()
        .min_h_0();
        let top = h_flex()
            .justify_between()
            .h(px(60.))
            .px(px(grid::GUTTER))
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(EDGE))
            .bg(rgba(0x1a1c1bd9))
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .size(px(32.))
                            .rounded(px(9.))
                            .justify_center()
                            .bg(rgb(PEARL))
                            .child(
                                Icon::new(IconName::GalleryVerticalEnd).text_color(rgb(0x202d24)),
                            ),
                    )
                    .child(Label::new("WPE Gallery").text_size(px(18.)).font_semibold()),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(muted("Display"))
                    .child(self.output_menu(cx)),
            );
        let controls = h_flex()
            .gap_3()
            .child(
                Switch::new("safe-filter")
                    .small()
                    .label("Safe only")
                    .checked(self.safe_only)
                    .tooltip("Show only reviewed, unchanged wallpapers")
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        this.safe_only = *on;
                        this.scroll_selection = true;
                        this.rebuild_visible(cx);
                    })),
            )
            .child(
                Button::new("reload")
                    .small()
                    .ghost()
                    .icon(IconName::Redo2)
                    .tooltip("Reload reviews")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_safety(cx))),
            );
        let heading = v_flex()
            .gap_1()
            .child(
                Label::new("Wallpaper library")
                    .text_size(px(30.))
                    .line_height(px(38.))
                    .font_semibold(),
            )
            .child(muted(format!(
                "{} of {} installed wallpapers",
                self.visible.len(),
                self.items.len()
            )));
        let compact = viewport.width < px(950.);
        let gallery = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .px(px(grid::GUTTER))
            .child(
                h_flex()
                    .when(compact, |s| s.flex_col().items_start().gap_2())
                    .justify_between()
                    .flex_shrink_0()
                    .py_4()
                    .child(heading)
                    .child(controls),
            )
            .when(self.visible.is_empty(), |s| {
                s.child(muted(
                    self.verification.empty_message(!self.items.is_empty()),
                ))
            })
            .when(!self.visible.is_empty(), |s| {
                s.when_some(self.verification.progress(), |s, msg| s.child(muted(msg)))
                    .when(matches!(self.verification, Verification::Failed(_)), |s| {
                        s.child(muted(self.verification.empty_message(true)))
                    })
            })
            .child(list)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .h(px(38.))
                    .gap_4()
                    .border_t_1()
                    .border_color(rgb(EDGE))
                    .child(shortcut(&["left", "right"], "Browse"))
                    .child(shortcut(&["tab"], "Display"))
                    .when(!compact, |s| s.child(shortcut(&["enter"], "Apply"))),
            );
        v_flex()
            .relative()
            .track_focus(&self.focus)
            .key_context("Gallery")
            .size_full()
            .bg(rgb(0x161717))
            .font_family("Inter")
            .text_sm()
            .text_color(rgb(INK))
            .on_action(|_: &Quit, _, cx| cx.quit())
            .on_action(cx.listener(|this, _: &Apply, _, cx| this.apply(cx)))
            .on_action(
                cx.listener(|this, _: &Next, _, cx| this.move_selection(grid::Step::Next, cx)),
            )
            .on_action(cx.listener(|this, _: &Previous, _, cx| {
                this.move_selection(grid::Step::Previous, cx)
            }))
            .on_action(cx.listener(|this, _: &Up, _, cx| this.move_selection(grid::Step::Up, cx)))
            .on_action(
                cx.listener(|this, _: &Down, _, cx| this.move_selection(grid::Step::Down, cx)),
            )
            .on_action(cx.listener(|this, _: &NextOutput, _, cx| {
                if !this.outputs.is_empty() {
                    this.output = (this.output + 1) % this.outputs.len();
                    cx.notify();
                }
            }))
            .child(
                img(surface::grain())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .child(top)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(gallery)
                    .child(self.details(cx)),
            )
    }
}

fn shortcut(keys: &[&str], label: &'static str) -> Div {
    h_flex()
        .gap_1()
        .children(
            keys.iter()
                .map(|key| Kbd::new(Keystroke::parse(key).expect("static shortcut"))),
        )
        .child(muted(label).ml_1().text_xs())
}
