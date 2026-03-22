mod app;
mod event;
mod ui;

use std::time::Duration;

use crossterm::event::KeyEventKind;

use crate::app::App;
use crate::event::{Event, EventHandler};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal).await;
    ratatui::restore();
    result
}

async fn run(terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
    let mut app = App::new();
    let mut events = EventHandler::new(Duration::from_millis(250));

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        match events.next().await {
            Some(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                app.handle_key(key);
            }
            Some(Event::Resize(_, _)) => {}
            Some(Event::Tick) => {}
            Some(Event::Key(_)) => {}
            None => break,
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
