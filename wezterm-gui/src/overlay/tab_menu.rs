use mux::termwiztermtab::TermWizTerminal;
use mux::tab::TabId;
use mux::Mux;
use promise::spawn::block_on;
use termwiz::input::{InputEvent, KeyCode, KeyEvent, Modifiers, MouseButtons, MouseEvent};
use termwiz::lineedit::*;
use termwiz::surface::{Change, Position};
use termwiz::terminal::Terminal;
use window::WindowOps;
use mux::MuxNotification;

enum TabMenuAction {
    Rename,
}

struct PromptHost {
    history: BasicHistory,
}

impl PromptHost {
    fn new() -> Self {
        Self {
            history: BasicHistory::default(),
        }
    }
}

impl LineEditorHost for PromptHost {
    fn history(&mut self) -> &mut dyn History {
        &mut self.history
    }

    fn resolve_action(
        &mut self,
        _event: &InputEvent,
        _editor: &mut LineEditor<'_>,
    ) -> Option<Action> {
        None
    }
}

fn render_menu(term: &mut TermWizTerminal) -> anyhow::Result<()> {
    let changes = vec![
        Change::ClearScreen(termwiz::color::ColorAttribute::Default),
        Change::CursorPosition {
            x: Position::Absolute(0),
            y: Position::Absolute(0),
        },
        Change::Text("Tab Menu\r\n".to_string()),
        termwiz::cell::AttributeChange::Reverse(true).into(),
        Change::Text(" 1. Rename\r\n".to_string()),
        termwiz::cell::AttributeChange::Reverse(false).into(),
    ];

    term.render(&changes)?;
    Ok(())
}

fn prompt_for_title(
    mut term: TermWizTerminal,
    tab_id: TabId,
    mux_window_id: mux::window::WindowId,
    initial_title: String,
    window: ::window::Window,
) -> anyhow::Result<()> {
    let _ = block_on(window.get_ime_open_status());
    let _ = block_on(window.set_ime_open_status(true));
    let _ = block_on(window.get_ime_open_status());
    term.render(&[
        Change::ClearScreen(termwiz::color::ColorAttribute::Default),
        Change::Text("Rename Tab\r\n".to_string()),
    ])?;

    let mut host = PromptHost::new();
    let mut editor = LineEditor::new(&mut term);
    editor.set_prompt("New title: ");
    if let Some(line) =
        editor.read_line_with_optional_initial_value(&mut host, Some(initial_title.as_str()))?
    {
        let mux = Mux::get();
        if let Some(tab) = mux.get_tab(tab_id) {
            tab.set_title(&line);
            mux.notify(MuxNotification::WindowInvalidated(mux_window_id));
        }
    }

    Ok(())
}

pub fn show_tab_menu(
    mut term: TermWizTerminal,
    tab_id: TabId,
    mux_window_id: mux::window::WindowId,
    window: ::window::Window,
) -> anyhow::Result<()> {
    term.set_raw_mode()?;
    term.no_grab_mouse_in_raw_mode();
    term.render(&[Change::Title("Tab Menu".to_string())])?;

    render_menu(&mut term)?;

    let action = loop {
        match term.poll_input(None)? {
            Some(InputEvent::Key(KeyEvent {
                key: KeyCode::Char('1'),
                ..
            }))
            | Some(InputEvent::Key(KeyEvent {
                key: KeyCode::Char('r'),
                ..
            }))
            | Some(InputEvent::Key(KeyEvent {
                key: KeyCode::Char('R'),
                ..
            })) => break Some(TabMenuAction::Rename),
            Some(InputEvent::Key(KeyEvent {
                key: KeyCode::Enter,
                ..
            })) => break Some(TabMenuAction::Rename),
            Some(InputEvent::Key(KeyEvent {
                key: KeyCode::UpArrow,
                ..
            }))
            | Some(InputEvent::Key(KeyEvent {
                key: KeyCode::Char('k'),
                modifiers: Modifiers::NONE,
            })) => {
                render_menu(&mut term)?;
            }
            Some(InputEvent::Key(KeyEvent {
                key: KeyCode::DownArrow,
                ..
            }))
            | Some(InputEvent::Key(KeyEvent {
                key: KeyCode::Char('j'),
                modifiers: Modifiers::NONE,
            })) => {
                render_menu(&mut term)?;
            }
            Some(InputEvent::Mouse(MouseEvent {
                y, mouse_buttons, ..
            })) if mouse_buttons == MouseButtons::LEFT && y == 1 => {
                break Some(TabMenuAction::Rename)
            }
            Some(InputEvent::Key(KeyEvent {
                key: KeyCode::Escape,
                ..
            }))
            | Some(InputEvent::Key(KeyEvent {
                key: KeyCode::Char('G'),
                modifiers: Modifiers::CTRL,
            }))
            | Some(InputEvent::Key(KeyEvent {
                key: KeyCode::Char('C'),
                modifiers: Modifiers::CTRL,
            }))
            | None => break None,
            _ => {}
        }
    };

    match action {
        Some(TabMenuAction::Rename) => {
            let title = Mux::get()
                .get_tab(tab_id)
                .map(|tab| tab.get_title())
                .unwrap_or_default();
            prompt_for_title(term, tab_id, mux_window_id, title, window)?;
        }
        None => {}
    }

    Ok(())
}
