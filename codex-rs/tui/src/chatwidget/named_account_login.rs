//! The named-account device sign-in lives only as long as its waiting view.

use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use futures::FutureExt;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use super::ChatWidget;
use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::BottomPaneView;
use crate::bottom_pane::CancellationEvent;
use crate::bottom_pane::ViewCompletion;
use crate::provider_named_accounts::ProviderAccountEvent;
use crate::render::renderable::Renderable;

pub(crate) struct NamedAccountLoginCode {
    pub(crate) verification_url: String,
    pub(crate) user_code: String,
}

impl ChatWidget {
    pub(crate) fn open_named_account_login(
        &mut self,
        account_label: String,
        codes: UnboundedReceiver<NamedAccountLoginCode>,
        task: JoinHandle<Result<String, String>>,
    ) {
        self.bottom_pane.show_view(Box::new(NamedAccountLoginView {
            account_label,
            app_event_tx: self.app_event_tx.clone(),
            codes,
            code: None,
            task,
            completion: None,
        }));
    }
}

struct NamedAccountLoginView {
    account_label: String,
    app_event_tx: AppEventSender,
    codes: UnboundedReceiver<NamedAccountLoginCode>,
    code: Option<NamedAccountLoginCode>,
    task: JoinHandle<Result<String, String>>,
    completion: Option<ViewCompletion>,
}

impl NamedAccountLoginView {
    fn poll_updates(&mut self) -> bool {
        if self.completion.is_some() {
            return false;
        }
        let mut changed = false;
        while let Ok(code) = self.codes.try_recv() {
            self.code = Some(code);
            changed = true;
        }
        if let Some(result) = (&mut self.task).now_or_never() {
            let result = result.unwrap_or_else(|_| {
                Err(format!(
                    "Sign-in of {} did not complete.",
                    self.account_label
                ))
            });
            self.completion = Some(ViewCompletion::Accepted);
            self.code = None;
            self.app_event_tx
                .send(AppEvent::ProviderAccount(ProviderAccountEvent::Finished {
                    result,
                }));
            changed = true;
        }
        changed
    }

    fn cancel(&mut self) {
        // A result already delivered by the task wins over a late Escape.
        self.poll_updates();
        if self.completion.is_some() {
            return;
        }
        self.task.abort();
        self.codes.close();
        self.code = None;
        self.completion = Some(ViewCompletion::Cancelled);
        self.app_event_tx
            .send(AppEvent::ProviderAccount(ProviderAccountEvent::Finished {
                result: Ok(format!("Cancelled sign-in of {}.", self.account_label)),
            }));
    }

    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        if self.completion.is_some() {
            return Vec::new();
        }
        let mut lines = vec![
            Line::from(format!("Sign in {}", self.account_label).bold()),
            Line::from(""),
        ];
        if let Some(code) = &self.code {
            lines.extend([
                Line::from("Open this link in your browser and sign in:"),
                Line::from(code.verification_url.clone().cyan().underlined()),
                Line::from(""),
                Line::from("Enter this one-time code:"),
                Line::from(code.user_code.clone().cyan().bold()),
                Line::from(""),
                Line::from("Device codes are a common phishing target. Never share this code.")
                    .dim(),
                Line::from("Waiting for the sign-in…").dim(),
            ]);
        } else {
            lines.push(Line::from("Requesting a one-time device code…").dim());
        }
        lines.push(Line::from("Esc or Ctrl-C cancels the sign-in.").dim());
        crate::wrapping::word_wrap_lines(&lines, width.max(1) as usize)
    }
}

impl Drop for NamedAccountLoginView {
    fn drop(&mut self) {
        // Closing the pane or application must not detach a pending sign-in.
        self.task.abort();
    }
}

impl BottomPaneView for NamedAccountLoginView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.code == KeyCode::Esc {
            self.cancel();
        }
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.cancel();
        CancellationEvent::Handled
    }

    fn prefer_esc_to_handle_key_event(&self) -> bool {
        true
    }

    fn is_complete(&self) -> bool {
        self.completion.is_some()
    }

    fn completion(&self) -> Option<ViewCompletion> {
        self.completion
    }

    fn pre_draw_tick(&mut self, _now: Instant) -> bool {
        self.poll_updates()
    }

    fn next_frame_delay(&self) -> Option<Duration> {
        self.completion
            .is_none()
            .then_some(Duration::from_millis(50))
    }
}

impl Renderable for NamedAccountLoginView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.lines(area.width)).render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.lines(width).len() as u16
    }
}

#[cfg(test)]
#[path = "named_account_login_tests.rs"]
mod tests;
