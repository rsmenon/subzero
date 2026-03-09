use std::time::{Duration, Instant};

use color_eyre::Result;
use crossterm::event::{self, Event, KeyEvent};
use tokio::sync::mpsc;

use crate::action::Action;

const TICK_RATE_ACTIVE: Duration = Duration::from_millis(16);
const TICK_RATE_IDLE: Duration = Duration::from_millis(250);
const IDLE_THRESHOLD: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Mouse,
    Tick,
    Action(Action),
    Resize,
}

pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<AppEvent>,
    pub action_tx: mpsc::UnboundedSender<AppEvent>,
}

impl EventHandler {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let action_tx = tx.clone();

        // Spawn crossterm event reader
        let event_tx = tx.clone();
        tokio::spawn(async move {
            let mut last_input = Instant::now();
            loop {
                let tick_rate = if last_input.elapsed() < IDLE_THRESHOLD {
                    TICK_RATE_ACTIVE
                } else {
                    TICK_RATE_IDLE
                };

                if event::poll(tick_rate).unwrap_or(false) {
                    match event::read() {
                        Ok(Event::Key(key)) => {
                            last_input = Instant::now();
                            if event_tx.send(AppEvent::Key(key)).is_err() {
                                break;
                            }
                        }
                        Ok(Event::Mouse(_)) => {
                            last_input = Instant::now();
                            if event_tx.send(AppEvent::Mouse).is_err() {
                                break;
                            }
                        }
                        Ok(Event::Resize(_, _)) => {
                            if event_tx.send(AppEvent::Resize).is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                } else {
                    // Tick event when no input
                    if event_tx.send(AppEvent::Tick).is_err() {
                        break;
                    }
                }
            }
        });

        Self { rx, action_tx }
    }

    pub async fn next(&mut self) -> Result<AppEvent> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| color_eyre::eyre::eyre!("Event channel closed"))
    }
}
