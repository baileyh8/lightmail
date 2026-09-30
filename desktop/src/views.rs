use crate::app::{date, MailDesktop, Page};
use crate::shortcuts::*;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, Textarea},
    Disableable, Selectable, Sizable,
};
use gpui_kit::component::{
    menu::{DropdownMenu, PopupMenuItem},
    Icon,
};
use gpui_kit::{prelude::*, *};
use lightmail_core::*;

const INK: u32 = 0x202724;
const MUTED: u32 = 0x69736e;
const ACCENT: u32 = 0x226451;
const LINE: u32 = 0xe7ebe8;
const SELECTED: u32 = 0xeaf2ed;
fn icon(name: &str) -> Icon {
    Icon::default()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(17.))
}
fn short_date(ts: i64) -> String {
    let now = chrono::Local::now();
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| {
            let d = d.with_timezone(&chrono::Local);
            d.format(if d.date_naive() == now.date_naive() {
                "%H:%M"
            } else {
                "%m月%d日"
            })
            .to_string()
        })
        .unwrap_or_default()
}
/// Kit sizes single-line inputs 2rem tall with fixed 8px vertical padding and a
/// 1.25rem line. At Lightmail's 13px rem that leaves under 10px for the text
/// line and cuts glyphs top and bottom, so single-line inputs get an explicit box.
fn line_input(input: Input, height: f32) -> Input {
    Styled::h(input.py(px(4.)), px(height))
}
fn column() -> Div {
    div().flex().flex_col().flex_shrink_0().min_w_0().min_h_0()
}
fn row() -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .min_w_0()
        .gap_2()
}
fn muted(text: impl Into<SharedString>) -> Div {
    div().text_sm().text_color(rgb(MUTED)).child(text.into())
}
fn title(text: impl Into<SharedString>) -> Div {
    div()
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}
fn folder_name(f: &Folder) -> String {
    match f.role.as_str() {
        "inbox" => "收件箱",
        "sent" => "已发送",
        "drafts" => "草稿",
        "trash" => "垃圾箱",
        "junk" => "垃圾邮件",
        _ => &f.name,
    }
    .into()
}

// Geometry probes are active only in isolated demo acceptance runs.
fn probe_marker(name: String, cx: &Context<MailDesktop>) -> impl IntoElement {
    let weak = cx.entity().downgrade();
    canvas(
        move |bounds, _, cx| {
            let _ = weak.update(cx, |s, _| {
                if s.acceptance.is_some() {
                    s.probes.insert(name, bounds);
                }
            });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
fn probe(name: &str, element: impl IntoElement, cx: &Context<MailDesktop>) -> Div {
    div()
        .relative()
        .child(element)
        .child(probe_marker(name.into(), cx))
}
impl MailDesktop {
    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = column()
            .id("sidebar-scroll")
            .flex_1()
            .overflow_y_scroll()
            .gap_1();
        for (label, scope) in [
            ("全部收件箱", "inbox"),
            ("已发送", "sent"),
            ("待发送", "outbox"),
            ("草稿", "drafts"),
            ("星标", "starred"),
        ] {
            let label = label.to_string();
            let scope = scope.to_string();
            let active =
                self.page == Page::Mail && self.account_id.is_empty() && self.scope == scope;
            list = list.child(
                div()
                    .id(SharedString::from(format!("scope-{scope}")))
                    .w_full()
                    .h(px(42.))
                    .px_4()
                    .flex()
                    .items_center()
                    .rounded_md()
                    .cursor_pointer()
                    .when(active, |v| v.bg(rgb(SELECTED)).text_color(rgb(ACCENT)))
                    .hover(|s| s.bg(rgb(SELECTED)))
                    .child(icon(match scope.as_str() {
                        "inbox" => "inbox",
                        "sent" => "plane",
                        "outbox" => "clock",
                        "drafts" => "file",
                        _ => "star",
                    }))
                    .gap_3()
                    .text_size(px(13.))
                    .child(label.clone())
                    .child(div().flex_1())
                    .child(muted({
                        let count = match scope.as_str() {
                            "inbox" => self
                                .folders
                                .iter()
                                .filter(|f| f.role == "inbox")
                                .map(|f| f.unread_count as usize)
                                .sum(),
                            "drafts" => self.drafts.iter().filter(|d| d.status == "draft").count(),
                            "outbox" => self
                                .drafts
                                .iter()
                                .filter(|d| {
                                    ["queued", "sending", "failed", "delivery_unknown"]
                                        .contains(&d.status.as_str())
                                })
                                .count(),
                            _ => 0,
                        };
                        if count == 0 {
                            String::new()
                        } else {
                            count.to_string()
                        }
                    }))
                    .on_click(cx.listener(move |s, _, w, cx| {
                        s.change_scope(
                            label.clone(),
                            String::new(),
                            String::new(),
                            scope.clone(),
                            w,
                            cx,
                        )
                    })),
            );
        }
        list = list.child(
            row()
                .mt_6()
                .px_3()
                .child(muted("邮箱").text_size(px(11.)))
                .child(div().flex_1())
                .child(
                    Button::new("add-mailbox")
                        .label("＋")
                        .ghost()
                        .on_click(cx.listener(|s, _, w, cx| s.open_accounts(None, w, cx))),
                ),
        );
        for account in &self.accounts {
            let a = account.clone();
            let id = a.id.clone();
            let expanded = self.expanded.contains(&id);
            let id2 = id.clone();
            let active = self.account_id == id;
            list = list.child(
                row()
                    .id(SharedString::from(format!("account-{id}")))
                    .h(px(44.))
                    .w_full()
                    .px_3()
                    .rounded_md()
                    .cursor_pointer()
                    .when(active, |v| v.bg(rgb(SELECTED)))
                    .hover(|s| s.bg(rgb(SELECTED)))
                    .child(
                        div()
                            .text_color(rgb(if self.errors.contains_key(&id) {
                                0xc19944
                            } else {
                                ACCENT
                            }))
                            .child("●"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .font_weight(FontWeight::MEDIUM)
                            .child(a.name.clone()),
                    )
                    .child(if self.syncing.contains(&id) { "·" } else { "" })
                    .child(
                        Button::new(SharedString::from(format!("expand-{id}")))
                            .label(if expanded { "⌄" } else { "›" })
                            .ghost()
                            .on_click(cx.listener(move |s, _, _, cx| {
                                cx.stop_propagation();
                                if !s.expanded.remove(&id2) {
                                    s.expanded.insert(id2.clone());
                                }
                                cx.notify();
                            })),
                    )
                    .on_click(cx.listener(move |s, _, w, cx| {
                        s.change_scope(
                            a.name.clone(),
                            a.id.clone(),
                            String::new(),
                            "inbox".into(),
                            w,
                            cx,
                        )
                    })),
            );
            if expanded {
                for folder in self.folders.iter().filter(|f| f.account_id == id) {
                    let f = folder.clone();
                    let label = folder_name(&f);
                    let active = self.folder_id == f.id;
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("folder-{}", f.id)))
                            .pl_8()
                            .pr_3()
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .rounded_md()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .when(active, |v| v.bg(rgb(SELECTED)).text_color(rgb(ACCENT)))
                            .hover(|s| s.bg(rgb(SELECTED)))
                            .child(label.clone())
                            .on_click(cx.listener(move |s, _, w, cx| {
                                s.change_scope(
                                    label.clone(),
                                    f.account_id.clone(),
                                    f.id.clone(),
                                    f.role.clone(),
                                    w,
                                    cx,
                                )
                            })),
                    );
                }
            }
        }
        column()
            .w(px(218.))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(0xf5f6f5))
            .border_r_1()
            .border_color(rgb(LINE))
            .p_3()
            .gap_3()
            .child(
                row()
                    .id("brand-drag")
                    .window_control_area(WindowControlArea::Drag)
                    .relative()
                    .h(px(70.))
                    .px_3()
                    .child(probe_marker("brand-drag".into(), cx))
                    .child(icon("plane").size(px(25.)).text_color(rgb(ACCENT)))
                    .child(
                        div()
                            .text_size(px(23.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("轻邮"),
                    ),
            )
            .child(probe(
                "compose",
                Button::new("compose")
                    .label("写邮件")
                    .icon(icon("compose"))
                    .outline()
                    .text_size(px(13.))
                    .w_full()
                    .h(px(34.))
                    .on_click(cx.listener(|s, _, w, cx| s.new_draft(ComposeMode::New, w, cx))),
                cx,
            ))
            .child(list)
            .when(self.demo, |v| {
                v.child(
                    Button::new("leave-demo")
                        .label("返回真实邮箱")
                        .ghost()
                        .on_click(cx.listener(|s, _, _, cx| s.switch_demo(false, cx))),
                )
            })
            .child(probe(
                "settings",
                Button::new("settings")
                    .label("设置")
                    .icon(icon("settings"))
                    .justify_start()
                    .text_size(px(13.))
                    .ghost()
                    .w_full()
                    .on_click(cx.listener(|s, _, w, cx| s.open_accounts(None, w, cx))),
                cx,
            ))
            .child(
                muted(if self.demo {
                    "示例模式 · 不连接真实邮箱"
                } else if self.syncing.is_empty() {
                    "本地优先 · 按需同步"
                } else {
                    "正在同步…"
                })
                .px_3(),
            )
    }
    fn mail_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let draft_list = ["drafts", "outbox"].contains(&self.scope.as_str());
        // The store's own rules decide which list a draft belongs to.
        let listed = |d: &Draft| {
            (self.account_id.is_empty() || d.account_id == self.account_id)
                && DraftState::of(d).is_some_and(|state| {
                    if self.scope == "drafts" {
                        state == DraftState::Draft
                    } else {
                        state.in_outbox()
                    }
                })
        };
        let count = if draft_list {
            self.drafts.iter().filter(|d| listed(d)).count()
        } else {
            self.messages.len()
        };
        let content = if draft_list {
            let mut list = column().id("draft-list").flex_1().overflow_y_scroll();
            for draft in self.drafts.iter().filter(|d| listed(d)) {
                let d = draft.clone();
                let id = d.id.clone();
                let state = DraftState::of(&d);
                let mut item =
                    column()
                        .p_4()
                        .gap_2()
                        .border_b_1()
                        .border_color(rgb(LINE))
                        .child(div().font_weight(FontWeight::MEDIUM).child(
                            if d.subject.is_empty() {
                                "无主题".into()
                            } else {
                                d.subject.clone()
                            },
                        ))
                        .child(muted(d.to.clone()))
                        .child(muted(match state {
                            Some(DraftState::Queued) => "等待发送 · 可撤销",
                            Some(DraftState::Sending) => "正在提交…",
                            Some(DraftState::Failed) => "发送失败",
                            Some(DraftState::DeliveryUnknown) => {
                                "结果待确认：请先在邮箱网页的已发送中核对"
                            }
                            Some(DraftState::Accepted) => "已提交",
                            _ => "本地草稿",
                        }));
                if !d.last_error.is_empty()
                    && matches!(
                        state,
                        Some(DraftState::Failed | DraftState::DeliveryUnknown)
                    )
                {
                    item = item.child(muted(d.last_error.clone()));
                }
                let mut actions = row();
                if state.is_some_and(DraftState::editable) {
                    let draft = d.clone();
                    actions = actions.child(
                        Button::new(SharedString::from(format!("edit-{id}")))
                            .label("继续编辑")
                            .on_click(
                                cx.listener(move |s, _, w, cx| s.edit_draft(draft.clone(), w, cx)),
                            ),
                    );
                }
                if state.is_some_and(DraftState::retryable) {
                    let id = id.clone();
                    actions = actions.child(
                        Button::new(SharedString::from(format!("retry-{id}")))
                            .label("重试发送")
                            .on_click(cx.listener(move |s, _, _, cx| s.retry_send(id.clone(), cx))),
                    );
                }
                if state.is_some_and(DraftState::withdrawable) {
                    let id = id.clone();
                    actions = actions.child(
                        Button::new(SharedString::from(format!("undo-{id}")))
                            .label("撤销发送")
                            .on_click(
                                cx.listener(move |s, _, _, cx| s.cancel_queue(id.clone(), cx)),
                            ),
                    );
                }
                // Only the user can settle an interrupted submission, after checking
                // the server; neither choice sends anything.
                if state.is_some_and(DraftState::resolvable) {
                    let delivered = id.clone();
                    let returned = id.clone();
                    actions = actions
                        .child(
                            Button::new(SharedString::from(format!("delivered-{id}")))
                                .label("已确认送达")
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    s.resolve_delivery(delivered.clone(), true, cx)
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("returned-{id}")))
                                .label("未送达，退回草稿")
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    s.resolve_delivery(returned.clone(), false, cx)
                                })),
                        );
                }
                if state == Some(DraftState::Draft) {
                    let id = id.clone();
                    actions = actions.child(
                        Button::new(SharedString::from(format!("delete-{id}")))
                            .label("删除草稿")
                            .ghost()
                            .on_click(
                                cx.listener(move |s, _, _, cx| s.delete_draft(id.clone(), cx)),
                            ),
                    );
                }
                item = item.child(actions);
                list = list.child(item);
            }
            list.into_any_element()
        } else {
            uniform_list(
                "mail-list",
                self.messages.len(),
                cx.processor(|s, range: std::ops::Range<usize>, _window, cx| {
                    range
                        .map(|index| {
                            let m = s.messages[index].clone();
                            let id = m.id.clone();
                            let selected = s.selected.as_ref().is_some_and(|v| v.id == id);
                            let account = s
                                .accounts
                                .iter()
                                .find(|a| a.id == m.account_id)
                                .map(|a| a.name.clone())
                                .unwrap_or_default();
                            column()
                                .id(SharedString::from(format!("message-{id}")))
                                .relative()
                                .child(probe_marker(format!("message-{index}"), cx))
                                .h(px(94.))
                                .w(px(355.))
                                .overflow_hidden()
                                .mx(px(5.))
                                .pl(px(27.))
                                .pr(px(12.))
                                .pt(px(13.))
                                .gap(px(5.))
                                .rounded(px(7.))
                                .cursor_pointer()
                                .when(selected, |v| v.bg(rgb(SELECTED)))
                                .hover(|v| v.bg(rgb(0xf3f6f3)))
                                .when(m.unread, |v| {
                                    v.child(
                                        div()
                                            .absolute()
                                            .left(px(12.))
                                            .top(px(21.))
                                            .size(px(6.))
                                            .rounded_full()
                                            .bg(rgb(ACCENT)),
                                    )
                                })
                                .child(
                                    row()
                                        .h(px(18.))
                                        .flex_shrink_0()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .line_clamp(1)
                                                .text_size(px(13.))
                                                .font_weight(if m.unread {
                                                    FontWeight::SEMIBOLD
                                                } else {
                                                    FontWeight::MEDIUM
                                                })
                                                .child(if m.from_name.is_empty() {
                                                    m.from_address
                                                } else {
                                                    m.from_name
                                                }),
                                        )
                                        .child(muted(short_date(m.timestamp)).text_size(px(11.))),
                                )
                                .child(
                                    div()
                                        .h(px(18.))
                                        .flex_shrink_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .line_clamp(1)
                                        .text_size(px(12.))
                                        .child(m.subject),
                                )
                                .child(
                                    row()
                                        .h(px(18.))
                                        .flex_shrink_0()
                                        .child(
                                            muted(if m.snippet.is_empty() {
                                                "正文将在打开时加载".into()
                                            } else {
                                                m.snippet
                                            })
                                            .w(px(238.))
                                            .flex_shrink_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .line_clamp(1),
                                        )
                                        .child(
                                            muted(account)
                                                .text_size(px(10.))
                                                .w(px(70.))
                                                .text_right()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .line_clamp(1),
                                        ),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(27.))
                                        .right(px(7.))
                                        .bottom_0()
                                        .h(px(0.5))
                                        .bg(rgb(LINE)),
                                )
                                .on_click(cx.listener(move |s, event: &ClickEvent, window, cx| {
                                    window.focus(&s.focus, cx);
                                    s.select(id.clone(), cx);
                                    if event.click_count() > 1 {
                                        s.focus_reading = true;
                                        cx.notify();
                                    }
                                }))
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .h_full()
            .flex_1()
            .into_any_element()
        };
        column()
            .w(px(365.))
            .bg(rgb(0xfcfcfb))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(rgb(LINE))
            .child(
                column()
                    .p_5()
                    .gap_4()
                    .child(
                        row()
                            .child(
                                div()
                                    .id("list-title-drag")
                                    .window_control_area(WindowControlArea::Drag)
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(21.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.title.clone())
                                    .child(probe_marker("list-title-drag".into(), cx)),
                            )
                            .child(
                                Button::new("refresh")
                                    .icon(icon("refresh"))
                                    .tooltip("刷新邮箱")
                                    .ghost()
                                    .on_click(cx.listener(|s, _, _, cx| s.refresh(cx))),
                            ),
                    )
                    .child(line_input(
                        Input::new(&self.fields["search"])
                            .small()
                            .prefix(icon("search"))
                            .bg(rgb(0xf3f5f3))
                            .text_size(px(12.)),
                        28.,
                    ))
                    .child(
                        row()
                            .child(
                                div()
                                    .id("all")
                                    .h(px(28.))
                                    .px(px(6.))
                                    .flex()
                                    .items_center()
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .border_b_2()
                                    .border_color(if !self.unread_only {
                                        rgb(ACCENT)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .text_color(if !self.unread_only {
                                        rgb(ACCENT)
                                    } else {
                                        rgb(MUTED)
                                    })
                                    .child("全部")
                                    .on_click(cx.listener(|s, _, _, cx| {
                                        s.unread_only = false;
                                        s.page_offset = 0;
                                        s.reload_search(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("unread")
                                    .h(px(28.))
                                    .px(px(6.))
                                    .flex()
                                    .items_center()
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .border_b_2()
                                    .border_color(if self.unread_only {
                                        rgb(ACCENT)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .text_color(if self.unread_only {
                                        rgb(ACCENT)
                                    } else {
                                        rgb(MUTED)
                                    })
                                    .child("未读")
                                    .on_click(cx.listener(|s, _, _, cx| {
                                        s.unread_only = true;
                                        s.page_offset = 0;
                                        s.reload_search(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(div().flex_1())
                            .child(muted(count.to_string())),
                    ),
            )
            .child(if count == 0 {
                column()
                    .flex_1()
                    .justify_center()
                    .items_center()
                    .gap_3()
                    .child(muted("暂无已同步邮件"))
                    .child(
                        Button::new("empty-refresh")
                            .label("重新同步")
                            .ghost()
                            .on_click(cx.listener(|s, _, _, cx| s.refresh(cx))),
                    )
                    .into_any_element()
            } else {
                content
            })
            .child(
                row()
                    .relative()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(probe_marker("list-footer".into(), cx))
                    .child(muted("最近邮件 · 正文按需读取").text_size(px(10.)))
                    .child(div().flex_1())
                    .when(self.page_offset > 0, |v| {
                        v.child(
                            Button::new("previous-page")
                                .label("上一页")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|s, _, _, cx| s.previous_page(cx))),
                        )
                    })
                    .child(
                        Button::new("older")
                            .label("加载更多")
                            .text_size(px(10.))
                            .text_color(rgb(ACCENT))
                            .ghost()
                            .small()
                            .disabled(draft_list || self.busy)
                            .on_click(cx.listener(|s, _, _, cx| s.older(cx))),
                    ),
            )
    }
    fn reader_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected.is_some();
        let body_ready = self.body.is_some();
        let weak = cx.entity().downgrade();
        let starred = self.selected.as_ref().is_some_and(|m| m.starred);
        let unread = self.selected.as_ref().is_some_and(|m| m.unread);
        let more = Button::new("reader-more")
            .icon(icon("more"))
            .ghost()
            .disabled(!selected)
            .dropdown_menu(move |menu, _, _| {
                let star = weak.clone();
                let seen = weak.clone();
                let layout = weak.clone();
                let reply = weak.clone();
                let forward = weak.clone();
                menu.item(
                    PopupMenuItem::new(if starred { "取消星标" } else { "加星标" }).on_click(
                        move |_, _, cx| {
                            let _ =
                                star.update(cx, |s, cx| s.mark_selected("starred", !starred, cx));
                        },
                    ),
                )
                .item(
                    PopupMenuItem::new(if unread {
                        "标记已读"
                    } else {
                        "标记未读"
                    })
                    .on_click(move |_, _, cx| {
                        let _ = seen.update(cx, |s, cx| s.mark_selected("seen", unread, cx));
                    }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("切换简化 HTML / 清爽阅读").on_click(move |_, _, cx| {
                        let _ = layout.update(cx, |s, cx| {
                            s.plain_reading = !s.plain_reading;
                            s.mode = ExportMode::Original;
                            s.reader_dirty = true;
                            cx.notify();
                        });
                    }),
                )
                .separator()
                .item(PopupMenuItem::new("回复全部").on_click(move |_, w, cx| {
                    let _ = reply.update(cx, |s, cx| s.new_draft(ComposeMode::ReplyAll, w, cx));
                }))
                .item(PopupMenuItem::new("转发").on_click(move |_, w, cx| {
                    let _ = forward.update(cx, |s, cx| s.new_draft(ComposeMode::Forward, w, cx));
                }))
            });
        let mut panel = column().flex_1().h_full().child(
            row()
                .h(px(64.))
                .flex_shrink_0()
                .px_5()
                .pr(px(crate::window_chrome::RESERVE.max(20.)))
                .when(self.focus_reading, |bar| {
                    bar.child(Button::new("back-list").label("返回列表").ghost().on_click(
                        cx.listener(|s, _, _, cx| {
                            s.focus_reading = false;
                            cx.notify();
                        }),
                    ))
                })
                .border_b_1()
                .border_color(rgb(LINE))
                .child(
                    Button::new("archive")
                        .icon(icon("archive"))
                        .tooltip("归档")
                        .ghost()
                        .disabled(!selected)
                        .on_click(cx.listener(|s, _, _, cx| s.move_selected("archive", cx))),
                )
                .child(
                    Button::new("trash")
                        .icon(icon("trash"))
                        .tooltip("移到垃圾箱")
                        .ghost()
                        .disabled(!selected)
                        .on_click(cx.listener(|s, _, _, cx| s.move_selected("trash", cx))),
                )
                .child(more)
                .child(
                    div()
                        .id("reader-toolbar-drag")
                        .window_control_area(WindowControlArea::Drag)
                        .relative()
                        .flex_1()
                        .h_full()
                        .child(probe_marker("reader-toolbar-drag".into(), cx)),
                )
                .child(
                    Button::new("translate")
                        .icon(icon("language"))
                        .outline()
                        .text_size(px(12.))
                        .label(if self.translating {
                            "取消翻译"
                        } else {
                            "全文翻译"
                        })
                        .disabled(!body_ready)
                        .on_click(cx.listener(|s, _, _, cx| {
                            if s.translating {
                                s.cancel_translation();
                                cx.notify();
                            } else {
                                s.translate(false, cx);
                            }
                        })),
                )
                .child(probe(
                    "copy-markdown",
                    Button::new("copy-markdown")
                        .label("复制 Markdown")
                        .icon(icon("copy"))
                        .text_size(px(12.))
                        .ghost()
                        .disabled(!body_ready)
                        .on_click(cx.listener(|s, _, _, cx| s.copy(cx))),
                    cx,
                )),
        );
        if let Some(message) = &self.selected {
            let account = self
                .accounts
                .iter()
                .find(|a| a.id == message.account_id)
                .map(|a| a.name.clone())
                .unwrap_or_default();
            let mut header = column()
                .px(px(36.))
                .pt_6()
                .pb_4()
                .gap_3()
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(message.subject.clone()),
                )
                .child(
                    row().child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded(px(3.))
                            .bg(rgb(SELECTED))
                            .text_color(rgb(ACCENT))
                            .text_size(px(11.))
                            .child(account),
                    ),
                )
                .child(
                    row()
                        .child(div().font_weight(FontWeight::MEDIUM).child(
                            if message.from_name.is_empty() {
                                message.from_address.clone()
                            } else {
                                message.from_name.clone()
                            },
                        ))
                        .child(muted(format!("<{}>", message.from_address)))
                        .child(div().flex_1())
                        .child(muted(date(message.timestamp))),
                )
                .child(muted(format!("收件人：{}", message.to_addresses)));
            if !message.cc_addresses.is_empty() {
                header = header.child(muted(format!("抄送：{}", message.cc_addresses)));
            }
            header = header.child(div().h(px(1.)).mt_3().mb_3().bg(rgb(LINE)));
            let mut controls = row().gap_2().child(
                Button::new("images")
                    .label(if self.images {
                        "隐藏外部图片"
                    } else {
                        "显示外部图片"
                    })
                    .ghost()
                    .small()
                    .disabled(!body_ready)
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.images = !s.images;
                        s.reader_dirty = true;
                        s.record("images");
                        cx.notify();
                    })),
            );
            if self.translation.is_some() {
                for (label, mode) in [
                    ("原文", ExportMode::Original),
                    ("译文", ExportMode::Translated),
                    ("双语", ExportMode::Bilingual),
                ] {
                    controls = controls.child(
                        Button::new(label)
                            .label(label)
                            .ghost()
                            .small()
                            .selected(
                                std::mem::discriminant(&mode) == std::mem::discriminant(&self.mode),
                            )
                            .on_click(cx.listener(move |s, _, _, cx| {
                                s.mode = mode;
                                s.reader_dirty = true;
                                cx.notify();
                            })),
                    );
                }
            }
            if !self.progress.is_empty() {
                controls = controls.child(muted(self.progress.clone()));
            }
            header = header.child(controls);
            panel = panel.child(header);
            let content = if self.loading {
                column()
                    .p_7()
                    .child(muted("正在读取正文…"))
                    .into_any_element()
            } else if let Some(error) = &self.reader_error {
                column()
                    .p_7()
                    .gap_3()
                    .child(muted(error.clone()))
                    .child(
                        Button::new("retry-body")
                            .label("重新读取")
                            .on_click(cx.listener(|s, _, _, cx| {
                                if let Some(m) = &s.selected {
                                    s.select(m.id.clone(), cx);
                                }
                            })),
                    )
                    .into_any_element()
            } else if let Some(document) = &self.reader {
                let mode = match self.mode {
                    ExportMode::Original => "original",
                    ExportMode::Translated => "translated",
                    ExportMode::Bilingual => "bilingual",
                };
                let layout = if self.plain_reading {
                    "markdown"
                } else {
                    "html"
                };
                div()
                    .relative()
                    .size_full()
                    .child(crate::reader::view(
                        &format!("{}:{mode}:{layout}", message.id),
                        document,
                        self.image_policy(),
                    ))
                    .child(probe_marker("reader-viewport".into(), cx))
                    .into_any_element()
            } else {
                column()
                    .p_7()
                    .child(muted("这封邮件暂无可显示正文"))
                    .into_any_element()
            };
            panel = panel.child(div().flex_1().min_h_0().px(px(36.)).child(content));
            if let Some(body) = &self.body {
                if !body.attachments.is_empty() {
                    let mut files = row().px(px(36.)).py_2();
                    for attachment in &body.attachments {
                        let a = attachment.clone();
                        files = files.child(
                            Button::new(SharedString::from(format!("attachment-{}", a.part_id)))
                                .label(a.filename.clone())
                                .ghost()
                                .small()
                                .on_click(
                                    cx.listener(move |s, _, _, cx| s.download(a.clone(), cx)),
                                ),
                        );
                    }
                    panel = panel.child(files);
                }
            }
            panel =
                panel.child(
                    row()
                        .px(px(36.))
                        .h(px(58.))
                        .child(
                            Button::new("reply")
                                .label("回复")
                                .icon(icon("reply"))
                                .outline()
                                .disabled(!body_ready)
                                .on_click(cx.listener(|s, _, w, cx| {
                                    s.new_draft(ComposeMode::Reply, w, cx)
                                })),
                        )
                        .child(
                            Button::new("reply-all")
                                .label("回复全部")
                                .ghost()
                                .disabled(!body_ready)
                                .on_click(cx.listener(|s, _, w, cx| {
                                    s.new_draft(ComposeMode::ReplyAll, w, cx)
                                })),
                        )
                        .child(
                            Button::new("forward")
                                .label("转发")
                                .ghost()
                                .disabled(!body_ready)
                                .on_click(cx.listener(|s, _, w, cx| {
                                    s.new_draft(ComposeMode::Forward, w, cx)
                                })),
                        ),
                );
        } else {
            panel = panel.child(
                column()
                    .flex_1()
                    .justify_center()
                    .items_center()
                    .gap_4()
                    .child(icon("envelope").size(px(48.)).text_color(rgb(0x80a899)))
                    .child(title("选一封邮件，慢慢读。"))
                    .child(muted("全文翻译与 Markdown 复制，随时在手边。")),
            );
        }
        panel
    }
    fn field(&self, key: &'static str, label: &str) -> impl IntoElement {
        column()
            .gap_2()
            .child(muted(label.to_string()))
            .child(match self.areas.get(key) {
                Some(area) => Textarea::new(area).into_any_element(),
                None => line_input(Input::new(&self.fields[key]), 30.).into_any_element(),
            })
    }
    fn settings_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = column().flex_1().h_full().child(
            row()
                .h(px(70.))
                .flex_shrink_0()
                .px(px(36.))
                .border_b_1()
                .border_color(rgb(LINE))
                .child(
                    div()
                        .text_size(px(22.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("设置"),
                )
                .child(div().flex_1())
                .child(
                    Button::new("accounts-tab")
                        .label("邮箱账号")
                        .ghost()
                        .when(self.page == Page::Accounts, |b| b.primary())
                        .on_click(cx.listener(|s, _, w, cx| s.open_accounts(None, w, cx))),
                )
                .child(probe(
                    "translation-tab",
                    Button::new("translation-tab")
                        .label("翻译")
                        .ghost()
                        .when(self.page == Page::Translation, |b| b.primary())
                        .on_click(cx.listener(|s, _, w, cx| {
                            s.page = Page::Translation;
                            s.load_translation_config(w, cx);
                            s.record("translation-page");
                            cx.notify();
                        })),
                    cx,
                ))
                .child(probe(
                    "storage-tab",
                    Button::new("storage-tab")
                        .label("存储")
                        .ghost()
                        .when(self.page == Page::Storage, |b| b.primary())
                        .on_click(cx.listener(|s, _, _, cx| s.open_storage(cx))),
                    cx,
                ))
                .child(div().flex_1())
                .child(probe(
                    "settings-done",
                    Button::new("settings-done")
                        .label("完成")
                        .outline()
                        .on_click(cx.listener(|s, _, _, cx| {
                            s.page = Page::Mail;
                            s.record("settings-close");
                            cx.notify();
                        })),
                    cx,
                )),
        );
        if self.page == Page::Accounts {
            let mut accounts = column()
                .id("settings-account-list")
                .overflow_y_scroll()
                .w(px(230.))
                .flex_shrink_0()
                .p_4()
                .gap_2()
                .border_r_1()
                .border_color(rgb(LINE))
                .bg(rgb(0xf5f6f5));
            accounts = accounts.child(
                row().child(muted("我的邮箱")).child(div().flex_1()).child(
                    Button::new("new-account")
                        .label("＋")
                        .ghost()
                        .on_click(cx.listener(|s, _, w, cx| {
                            s.open_accounts(None, w, cx);
                            s.adding_account = true;
                            cx.notify();
                        })),
                ),
            );
            for a in &self.accounts {
                let a = a.clone();
                let selected = self.editing.as_ref().is_some_and(|e| e.id == a.id);
                accounts = accounts.child(
                    column()
                        .id(SharedString::from(format!("settings-{}", a.id)))
                        .relative()
                        .child(probe_marker(format!("settings-{}", a.id), cx))
                        .p_3()
                        .gap_1()
                        .w_full()
                        .rounded_md()
                        .cursor_pointer()
                        .when(selected, |v| v.bg(rgb(SELECTED)))
                        .hover(|s| s.bg(rgb(SELECTED)))
                        .child(div().font_weight(FontWeight::MEDIUM).child(a.name))
                        .child(muted(a.address).text_xs())
                        .on_click(cx.listener(move |s, _, w, cx| {
                            s.open_accounts(Some(a.id.clone()), w, cx)
                        })),
                );
            }
            let mut form = column()
                .id("account-form")
                .flex_1()
                .overflow_y_scroll()
                .p_7()
                .gap_5()
                .child(title(if self.editing.is_some() {
                    "邮箱账号"
                } else {
                    "添加邮箱"
                }));
            let mut providers = row();
            for (label, p) in [
                ("Gmail", "gmail"),
                ("163 邮箱", "163"),
                ("QQ 邮箱", "qq"),
                ("其他 IMAP", "custom"),
            ] {
                providers = providers.child(
                    Button::new(label)
                        .label(label)
                        .when(self.provider == p, |b| b.primary())
                        .on_click(cx.listener(move |s, _, w, cx| s.apply_preset(p.into(), w, cx))),
                );
            }
            form = form
                .child(providers)
                .child(self.field("name", "显示名称"))
                .child(self.field("address", "邮箱地址"));
            if self.provider == "gmail" {
                form = form.child(
                    row()
                        .child(
                            Button::new("oauth-mode")
                                .label("Google 登录")
                                .selected(self.oauth)
                                .on_click(cx.listener(|s, _, window, cx| {
                                    // A password typed for app-password sign-in must not
                                    // travel with a Google sign-in.
                                    if !s.oauth {
                                        s.set("password", "", window, cx);
                                    }
                                    s.oauth = true;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("password-mode")
                                .label("应用专用密码")
                                .selected(!self.oauth)
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.oauth = false;
                                    cx.notify();
                                })),
                        ),
                );
            }
            if self.oauth {
                form = form
                    .child(self.field("client_id", "Google OAuth Desktop Client ID"))
                    .child(self.field("client_secret", "Client Secret（已有配置可留空）"));
            } else {
                form = form
                    .child(self.field("password", "客户端授权码（已保存可留空）"))
                    .child(muted(
                        "QQ / 163 需在邮箱网页开启 IMAP/SMTP，并生成客户端授权码。",
                    ));
            }
            form = form
                .child(
                    row()
                        .child(div().flex_1().child(self.field("imap", "IMAP 服务器")))
                        .child(div().w(px(100.)).child(self.field("imap_port", "端口"))),
                )
                .child(
                    row()
                        .child(div().flex_1().child(self.field("smtp", "SMTP 服务器")))
                        .child(div().w(px(100.)).child(self.field("smtp_port", "端口"))),
                )
                .child(
                    row()
                        .child(
                            Button::new("enabled")
                                .label(if self.enabled {
                                    "✓ 自动同步"
                                } else {
                                    "自动同步已关闭"
                                })
                                .ghost()
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.enabled = !s.enabled;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("sent-mode")
                                .label(if self.append_sent {
                                    "由轻邮保存已发送副本"
                                } else {
                                    "由服务器保存已发送副本"
                                })
                                .ghost()
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.append_sent = !s.append_sent;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    row()
                        .child(
                            Button::new("account-save")
                                .label(if self.oauth {
                                    "使用 Google 登录"
                                } else {
                                    "连接并保存"
                                })
                                .primary()
                                .disabled(self.busy)
                                .on_click(cx.listener(|s, _, _, cx| s.save_account(true, cx))),
                        )
                        .when(self.editing.is_some(), |v| {
                            v.child(
                                Button::new("account-update")
                                    .label("保存设置")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|s, _, _, cx| s.save_account(false, cx))),
                            )
                        }),
                );
            if self.editing.is_some() {
                form = form.child(div().h(px(1.)).bg(rgb(LINE))).child(muted(
                    "移除会清除此设备上的邮箱、缓存、本地草稿和凭证。服务器邮件不会删除。",
                ));
                form = form.child(
                    row()
                        .child(
                            Button::new("account-remove")
                                .label(if self.removal {
                                    "确认移除邮箱"
                                } else {
                                    "移除邮箱"
                                })
                                .danger()
                                .disabled(self.busy)
                                .on_click(cx.listener(|s, _, _, cx| {
                                    if s.removal {
                                        s.remove_account(cx);
                                    } else {
                                        s.removal = true;
                                        cx.notify();
                                    }
                                })),
                        )
                        .when(self.removal, |v| {
                            v.child(Button::new("cancel-remove").label("取消").ghost().on_click(
                                cx.listener(|s, _, _, cx| {
                                    s.removal = false;
                                    cx.notify();
                                }),
                            ))
                        }),
                );
            }
            let form =
                if self.editing.is_none() && !self.adding_account && !self.accounts.is_empty() {
                    column()
                        .flex_1()
                        .h_full()
                        .p(px(30.))
                        .gap_5()
                        .child(
                            div()
                                .text_size(px(24.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("连接你的邮箱"),
                        )
                        .child(muted("每个账号独立同步，聚合查看时仍清楚标明来源。"))
                        .child(
                            row().child(
                                Button::new("start-add-account")
                                    .label("添加邮箱")
                                    .primary()
                                    .on_click(cx.listener(|s, _, _, cx| {
                                        s.adding_account = true;
                                        cx.notify();
                                    })),
                            ),
                        )
                        .into_any_element()
                } else {
                    form.h_full().into_any_element()
                };
            root = root.child(
                row()
                    .items_start()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .gap_0()
                    .child(accounts.h_full())
                    .child(form),
            );
        } else if self.page == Page::Storage {
            // Figures arrive from a background read; never show zeros meanwhile.
            let (counts, sizes) = match self.storage.as_ref() {
                Some(i) => (
                    format!(
                        "已同步 {} 封摘要 · 已缓存 {} 封正文",
                        i.message_count, i.body_count
                    ),
                    format!(
                        "数据库 {:.1} MiB · 正文缓存 {:.1} MiB",
                        i.database_bytes as f64 / 1048576.,
                        i.cache_bytes as f64 / 1048576.
                    ),
                ),
                None => ("正在读取存储信息…".into(), String::new()),
            };
            root=root.child(column().p_7().gap_5().child(title("本地存储"))
                .child(muted(counts))
                .child(muted(sizes))
                .child(muted("每个邮箱预加载最新 20 封。新邮件到达后释放更早的正文，保留摘要；旧正文和附件按需读取。"))
                .child(row().child(Button::new("clear-cache").label("清理正文缓存").disabled(self.busy).on_click(cx.listener(|s,_,_,cx|s.clear_cache(cx)))).child(Button::new("import-mail").label("导入 .eml 邮件").on_click(cx.listener(|s,_,_,cx|s.import_mail(cx)))))
                .child(muted("账号授权码和 API Key 保存在 Windows 凭据管理器。卸载应用时保留邮件数据，避免误删。")));
        } else {
            let mut profiles = column()
                .w(px(230.))
                .h_full()
                .flex_shrink_0()
                .p_4()
                .gap_2()
                .bg(rgb(0xf5f6f5))
                .border_r_1()
                .border_color(rgb(LINE))
                .child(row().child(muted("翻译服务")).child(div().flex_1()).child(
                    Button::new("add-translation").label("＋").ghost().on_click(
                        cx.listener(|s, _, w, cx| s.select_translation_config(None, w, cx)),
                    ),
                ));
            for config in self.configurations.iter().filter(|c| c.engine == "llm") {
                let c = config.clone();
                profiles = profiles.child(
                    column()
                        .id(SharedString::from(format!("config-{}", c.id)))
                        .p_3()
                        .gap_1()
                        .rounded_md()
                        .cursor_pointer()
                        .when(
                            self.configuration
                                .as_ref()
                                .is_some_and(|current| current.id == c.id),
                            |v| v.bg(rgb(SELECTED)),
                        )
                        .hover(|v| v.bg(rgb(SELECTED)))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(c.name.clone()),
                        )
                        .child(muted(c.model.clone()).text_size(px(10.)))
                        .on_click(cx.listener(move |s, _, w, cx| {
                            s.select_translation_config(Some(c.id.clone()), w, cx)
                        })),
                );
            }
            profiles = profiles
                .child(div().flex_1())
                .child(muted("只在你点击翻译时请求。\n译文缓存到本机。").text_size(px(11.)));
            let form=column().id("translation-form").flex_1().overflow_y_scroll().p_7().gap_5().child(title("全文翻译"))
                .child(muted("使用 OpenAI Chat Completions 兼容服务。Windows 版使用 LLM 翻译；系统语言包功能仅适用于 macOS。"))
                .child(self.field("translation_name","配置名称")).child(self.field("base_url","Base URL")).child(self.field("api_key","API Key（已有配置可留空）")).child(self.field("model","Model")).child(self.field("language","目标语言")).child(self.field("glossary","术语表"))
                .child(row().child(Button::new("stream").label(if self.config_stream{"✓ 流式响应"}else{"非流式响应"}).ghost().on_click(cx.listener(|s,_,_,cx|{s.config_stream=!s.config_stream;cx.notify();}))).child(Button::new("json-format").label(if self.config_json{"✓ JSON 模式"}else{"提示词 JSON（兼容性优先）"}).ghost().on_click(cx.listener(|s,_,_,cx|{s.config_json=!s.config_json;cx.notify();}))))
                .child(muted("仅在主动翻译时发送当前邮件主题与正文，附件不会上传。测试连接只发送合成文本。"))
                .child(row().child(Button::new("save-translation").label("保存并设为默认").primary().disabled(self.busy).on_click(cx.listener(|s,_,_,cx|s.save_translation(false,cx)))).child(Button::new("test-translation").label("测试连接").disabled(self.busy).on_click(cx.listener(|s,_,_,cx|s.save_translation(true,cx)))));
            root = root.child(
                row()
                    .gap_0()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(profiles)
                    .child(form.h_full()),
            );
        }
        root.child(
            row()
                .h(px(38.))
                .flex_shrink_0()
                .px_6()
                .border_t_1()
                .border_color(rgb(LINE))
                .child(muted("轻邮 Lightmail"))
                .child(div().flex_1())
                .child(
                    Button::new("privacy")
                        .label("隐私政策")
                        .ghost()
                        .small()
                        .on_click(|_, _, cx| cx.open_url("https://lightmail.sohym.com/privacy/")),
                )
                .child(
                    Button::new("terms")
                        .label("使用条款")
                        .ghost()
                        .small()
                        .on_click(|_, _, cx| cx.open_url("https://lightmail.sohym.com/terms/")),
                ),
        )
    }
    fn window_chrome(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let mut chrome = div()
            .flex()
            .absolute()
            .top_0()
            .right_0()
            .h(px(crate::window_chrome::HEIGHT))
            // Header drag areas painted earlier must never claim the controls.
            .occlude();
        for (name, control) in crate::window_chrome::controls(window, cx) {
            chrome = chrome.child(probe(name, control, cx));
        }
        chrome
    }
    fn welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        column()
            .relative()
            .flex_1()
            .h_full()
            .items_center()
            .justify_center()
            .gap_5()
            .child(
                div()
                    .id("welcome-drag")
                    .window_control_area(WindowControlArea::Drag)
                    .absolute()
                    .top_0()
                    .left_0()
                    .right(px(crate::window_chrome::WIDTH))
                    .h(px(64.)),
            )
            .child(icon("plane").size(px(48.)).text_color(rgb(ACCENT)))
            .child(
                div()
                    .text_size(px(34.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_center()
                    .child("邮件归于一处，\n专注每一次沟通。"),
            )
            .child(muted("连接 Gmail、163 和 QQ，轻快地收发、阅读与翻译。"))
            .child(
                Button::new("welcome-add")
                    .label("连接邮箱")
                    .primary()
                    .on_click(cx.listener(|s, _, w, cx| {
                        s.open_accounts(None, w, cx);
                        s.adding_account = true;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("try-demo")
                    .label("先体验示例")
                    .ghost()
                    .on_click(cx.listener(|s, _, _, cx| s.switch_demo(true, cx))),
            )
    }
    fn compose_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let account = self
            .draft
            .as_ref()
            .and_then(|d| self.accounts.iter().find(|a| a.id == d.account_id));
        let sender = account
            .map(|a| format!("{} <{}>", a.name, a.address))
            .unwrap_or_default();
        let accounts = self.accounts.clone();
        let view = cx.entity().downgrade();
        let sender_menu = Button::new("compose-sender")
            .label(sender)
            .ghost()
            .dropdown_menu(move |mut menu, _, _| {
                for account in &accounts {
                    let id = account.id.clone();
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{} <{}>", account.name, account.address))
                            .on_click(move |_, _, cx| {
                                let _ = view.update(cx, |s, cx| {
                                    if let Some(d) = &mut s.draft {
                                        d.account_id = id.clone();
                                    }
                                    s.persist_compose(cx);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            });
        let recipient = |key: &'static str, label: &'static str| {
            row()
                .h(px(44.))
                .border_b_1()
                .border_color(rgb(LINE))
                .child(muted(label).w(px(60.)).flex_shrink_0())
                .child(line_input(
                    Input::new(&self.fields[key]).appearance(false).flex_1(),
                    30.,
                ))
        };
        column()
            .flex_1()
            .h_full()
            .child(
                row()
                    .h(px(70.))
                    .px_6()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        div()
                            .text_size(px(21.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("写邮件"),
                    )
                    .child(div().flex_1())
                    .child(muted("草稿保存在本机").text_size(px(11.)))
                    .child(probe(
                        "save-draft",
                        Button::new("save-draft")
                            .label("关闭")
                            .outline()
                            .on_click(cx.listener(|s, _, _, cx| s.save_draft(false, cx))),
                        cx,
                    )),
            )
            .child(
                column()
                    .px(px(26.))
                    .child(
                        row()
                            .h(px(44.))
                            .border_b_1()
                            .border_color(rgb(LINE))
                            .child(muted("发件人").w(px(60.)))
                            .child(sender_menu),
                    )
                    .child(
                        recipient("to", "收件人").child(
                            Button::new("extra-recipients")
                                .label(if self.extra_recipients {
                                    "收起"
                                } else {
                                    "抄送 / 密送"
                                })
                                .ghost()
                                .small()
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.extra_recipients = !s.extra_recipients;
                                    cx.notify();
                                })),
                        ),
                    )
                    .when(self.extra_recipients, |v| {
                        v.child(recipient("cc", "抄送"))
                            .child(recipient("bcc", "密送"))
                    })
                    .child(recipient("subject", "主题")),
            )
            .child(
                div().flex_1().min_h_0().px_6().py_4().child(
                    Textarea::new(&self.areas["draft_body"])
                        .appearance(false)
                        .text_size(px(15.))
                        .h_full(),
                ),
            )
            .when(!self.attachments.is_empty(), |v| {
                v.child(
                    row()
                        .id("compose-attachments")
                        .h(px(44.))
                        .overflow_x_scroll()
                        .px_6()
                        .children(self.attachments.iter().map(|path| {
                            let path = path.clone();
                            Button::new(SharedString::from(path.clone()))
                                .label(format!(
                                    "{} ×",
                                    std::path::Path::new(&path)
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                ))
                                .small()
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    s.attachments.retain(|p| p != &path);
                                    s.persist_compose(cx);
                                    cx.notify();
                                }))
                        })),
                )
            })
            .child(
                row()
                    .h(px(74.))
                    .px_6()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(
                        Button::new("send")
                            .label("发送")
                            .icon(icon("plane"))
                            .primary()
                            .disabled(self.busy)
                            .on_click(cx.listener(|s, _, _, cx| s.save_draft(true, cx))),
                    )
                    .child(
                        Button::new("add-attachment")
                            .label("添加附件")
                            .ghost()
                            .on_click(cx.listener(|s, _, _, cx| {
                                if let Some(files) = rfd::FileDialog::new().pick_files() {
                                    for path in files {
                                        let path = path.to_string_lossy().into_owned();
                                        if !s.attachments.contains(&path) {
                                            s.attachments.push(path);
                                        }
                                    }
                                    s.persist_compose(cx);
                                    cx.notify();
                                }
                            })),
                    )
                    .child(div().flex_1())
                    .child(muted("发送后有 5 秒撤销时间").text_size(px(11.))),
            )
    }
}
impl Render for MailDesktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.clear_secrets {
            for key in ["password", "client_secret", "api_key"] {
                self.set(key, "", window, cx);
            }
            self.clear_secrets = false;
        }
        self.service.set_active(window.is_window_active());
        self.update_reader(cx);
        let settings = matches!(
            self.page,
            Page::Accounts | Page::Translation | Page::Storage
        );
        let composing = self.page == Page::Compose;
        let content = if self.accounts.is_empty() {
            self.welcome(cx).into_any_element()
        } else {
            row()
                .items_start()
                .gap_0()
                .flex_1()
                .min_h_0()
                .when(!self.focus_reading, |v| v.child(self.mail_list(cx)))
                .child(self.reader_panel(cx))
                .into_any_element()
        };
        div()
            .id("lightmail-root")
            .track_focus(&self.focus)
            .key_context("Lightmail")
            .on_action(cx.listener(|s, _: &NewMessage, w, cx| s.new_draft(ComposeMode::New, w, cx)))
            .on_action(cx.listener(|s, _: &FocusSearch, w, cx| {
                s.fields["search"].update(cx, |input, cx| input.focus(w, cx));
            }))
            .on_action(cx.listener(|s, _: &RefreshMail, _, cx| s.refresh(cx)))
            .on_action(cx.listener(|s, _: &Reply, w, cx| s.new_draft(ComposeMode::Reply, w, cx)))
            .on_action(cx.listener(|s, _: &Translate, _, cx| s.translate(false, cx)))
            .on_action(cx.listener(|s, _: &CopyMarkdown, _, cx| s.copy(cx)))
            .on_action(cx.listener(|s, _: &Settings, w, cx| s.open_accounts(None, w, cx)))
            .on_action(cx.listener(|s, _: &ImportMail, _, cx| s.import_mail(cx)))
            .on_action(cx.listener(|s, _: &ClosePanel, _, cx| {
                s.persist_compose(cx);
                s.page = Page::Mail;
                s.focus_reading = false;
                cx.notify();
            }))
            .relative()
            .size_full()
            .font_family(if cfg!(windows) {
                "Microsoft YaHei UI"
            } else {
                ".SystemUIFont"
            })
            .text_size(px(13.))
            .text_color(rgb(INK))
            .bg(rgb(0xffffff))
            .child(
                row()
                    .size_full()
                    .items_start()
                    .gap_0()
                    .child(self.sidebar(cx))
                    .child(column().flex_1().h_full().child(content)),
            )
            .when(settings || composing, |root| {
                // The dimmed margin moves the window and keeps the dialog clear of the
                // controls; the dialog occludes it, so its own clicks stay clicks.
                root.child(
                    div()
                        .id("overlay-drag")
                        .window_control_area(WindowControlArea::Drag)
                        .absolute()
                        .inset_0()
                        .p(px(crate::window_chrome::HEIGHT))
                        .bg(rgba(0x20272433))
                        .occlude()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .w(px(if composing { 780. } else { 960. }))
                                .max_w_full()
                                .h(px(if composing { 670. } else { 700. }))
                                .max_h_full()
                                .relative()
                                .occlude()
                                .rounded(px(16.))
                                .overflow_hidden()
                                .bg(rgb(0xffffff))
                                .border_1()
                                .border_color(rgb(LINE))
                                .shadow_lg()
                                .child(if composing {
                                    self.compose_panel(cx).into_any_element()
                                } else {
                                    self.settings_panel(cx).into_any_element()
                                })
                                .child(probe_marker("overlay-panel".into(), cx)),
                        ),
                )
            })
            .when(!self.status.is_empty(), |root| {
                root.child(
                    div()
                        .absolute()
                        .bottom(px(16.))
                        .right(px(16.))
                        // Above the dimmed margin, the note keeps its own clicks.
                        .occlude()
                        .max_w(px(600.))
                        .rounded_md()
                        .bg(rgb(SELECTED))
                        .border_1()
                        .border_color(rgb(LINE))
                        .px_4()
                        .py_2()
                        .child(
                            row().child(muted(self.status.clone())).child(
                                Button::new("dismiss-status")
                                    .label("×")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|s, _, _, cx| {
                                        s.status.clear();
                                        cx.notify();
                                    })),
                            ),
                        ),
                )
            })
            // Last, so the controls stay above dialogs and notes.
            .child(self.window_chrome(window, cx))
    }
}
