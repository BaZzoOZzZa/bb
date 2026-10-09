use bb_client::ServerEndpoint;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::widgets::{Block, Paragraph};

pub struct App {
    endpoint: Option<ServerEndpoint>,
    quit: bool,
}

impl App {
    pub fn new(endpoint: Option<ServerEndpoint>) -> Self {
        Self {
            endpoint,
            quit: false,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            _ => {}
        }
    }

    pub fn render(&self, frame: &mut Frame) {
        let server = self
            .endpoint
            .as_ref()
            .map_or_else(|| "auto".to_string(), |e| e.base().to_string());
        let body = format!("bb-tui\n\nserver: {server}\n\npress q to quit");
        let widget = Paragraph::new(body)
            .alignment(Alignment::Center)
            .block(Block::bordered().title(" bb "));
        frame.render_widget(widget, frame.area());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn quits_on_q_and_ctrl_c_only() {
        let mut app = App::new(None);
        app.handle_key(KeyEvent::from(KeyCode::Char('x')));
        assert!(!app.should_quit());
        app.handle_key(KeyEvent::from(KeyCode::Char('q')));
        assert!(app.should_quit());

        let mut app = App::new(None);
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit());
    }

    #[test]
    fn placeholder_frame_shows_server() {
        let endpoint = ServerEndpoint::parse("http://127.0.0.1:4000").ok();
        let app = App::new(endpoint);
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).expect("terminal");
        terminal.draw(|frame| app.render(frame)).expect("draw");
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("server: http://127.0.0.1:4000/"));
        assert!(text.contains("press q to quit"));
    }
}
