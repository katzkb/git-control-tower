use std::time::Duration;

use crossterm::event::{self, Event as CtEvent, KeyEvent};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Tick,
    #[allow(dead_code)]
    Resize(u16, u16),
}

pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            loop {
                if event::poll(tick_rate).unwrap_or(false) {
                    let send_result = match event::read() {
                        Ok(CtEvent::Key(key)) => tx.send(Event::Key(key)),
                        Ok(CtEvent::Resize(w, h)) => tx.send(Event::Resize(w, h)),
                        _ => continue,
                    };
                    if send_result.is_err() {
                        return;
                    }
                } else if tx.send(Event::Tick).is_err() {
                    return;
                }
            }
        });

        Self { rx }
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}
