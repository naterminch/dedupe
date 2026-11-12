//! Toast layer over `nsh/base`'s headless toast stack.
//!
//! Workers and dialogs cannot reach the view directly, so they call
//! [`notify`]; the manager lives in a global signal the interval-driven
//! layer renders from, keyed by toast id so transitions survive re-renders.

use std::time::Duration;

use nsh_base::{
    Anchor, ToastManager, ToastMotion, ToastOptions, ToastStackState, ToastTransitionStatus, toast,
    toast_stack,
};
use nsh_core::prelude::*;
use nsh_core::time::now;

use crate::ui::palette::{colors, radius, space, text};
use crate::ui::widgets::{motion, timing};

/// How long a toast stays on screen before it dismisses itself.
const TOAST_TIMEOUT: Duration = Duration::from_secs(6);

/// One toast's content: the message and whether it reports a failure.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ToastMsg {
    text: SharedString,
    error: bool,
}

/// The shared toast store: the headless manager plus an id counter for
/// `push`.
#[derive(Clone)]
struct Toasts {
    manager: ToastManager<u64, ToastMsg>,
    next_id: u64,
}

impl Default for Toasts {
    fn default() -> Self {
        Self {
            manager: ToastManager::new(ToastMotion::sonner()),
            next_id: 0,
        }
    }
}

fn toasts() -> Signal<Toasts> {
    global::<Toasts>()
}

/// Queue a toast from anywhere (worker callbacks included). `error` tints
/// the message with the danger colour.
pub(crate) fn notify(message: String, error: bool) {
    toasts().update(|t| {
        t.next_id += 1;
        t.manager.push(
            t.next_id,
            ToastMsg {
                text: message.into(),
                error,
            },
            ToastOptions::timeout(Some(TOAST_TIMEOUT)),
            now(),
        );
    });
}

pub(crate) fn toast_layer() -> impl IntoElement {
    let stack = ToastStackState::new();
    // Advance the lifecycle on a timer; timeouts pause while the stack is
    // expanded under the pointer.
    set_interval(Duration::from_millis(50), move || {
        let expanded = stack.is_expanded();
        toasts().update(|t| {
            t.manager.advance(now(), expanded);
        });
    });
    view()
        .absolute()
        .left(px(0.0))
        .right(px(0.0))
        .bottom(px(space::XL))
        .flex_row()
        .justify_center()
        .child(
            toast_stack()
                .placement(Anchor::BottomCenter)
                .state(stack)
                .w(px(380.0))
                .child(each(
                    || {
                        toasts().with(|t| {
                            t.manager
                                .visible(3)
                                .map(|(id, msg, status)| (*id, msg.clone(), status))
                                .collect::<Vec<_>>()
                        })
                    },
                    |(id, _, _): &(u64, ToastMsg, ToastTransitionStatus)| *id,
                    |item, _| {
                        let (error, message) =
                            item.with(|(_, msg, _)| (msg.error, msg.text.clone()));
                        let label = message.clone();
                        toast()
                            .transition_status(move || item.with(|(_, _, status)| *status))
                            .accessibility_label(move || Some(label.clone()))
                            .on_dismiss_request({
                                let id = item.with(|(id, _, _)| *id);
                                move || {
                                    toasts().update(|t| {
                                        t.manager.dismiss(&id, now());
                                    });
                                }
                            })
                            .rounded(px(radius::CARD))
                            .px(px(space::LG))
                            .py(px(space::MD))
                            .bg(colors::CARD)
                            .border_1()
                            .border_color(if error {
                                colors::DANGER
                            } else {
                                colors::BORDER
                            })
                            .transition_all()
                            .duration(motion(timing::SLOW))
                            .starting_class("translate-y-3 opacity-0")
                            .ending_class("opacity-0")
                            .child(
                                text(move || message.clone())
                                    .text_size(px(text::CAPTION))
                                    .text_color(if error {
                                        colors::DANGER_TEXT
                                    } else {
                                        colors::TEXT
                                    }),
                            )
                    },
                )),
        )
}
