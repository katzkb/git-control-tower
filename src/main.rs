use std::io;

use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, Paragraph},
};

fn main() -> anyhow::Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;

    let terminal = ratatui::init();
    let result = run(terminal);

    ratatui::restore();
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run(mut terminal: DefaultTerminal) -> anyhow::Result<()> {
    loop {
        terminal.draw(ui)?;

        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            return Ok(());
        }
    }
}

fn ui(frame: &mut Frame) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(frame.area());

    let title = Paragraph::new(Text::raw("Git Control Tower"))
        .block(Block::default().borders(Borders::ALL).title(" gct "));
    frame.render_widget(title, chunks[0]);

    let status = Paragraph::new("Press q to quit").style(Style::default().fg(Color::DarkGray));
    frame.render_widget(status, chunks[1]);
}
