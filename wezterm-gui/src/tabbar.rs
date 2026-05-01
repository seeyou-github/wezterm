use crate::termwindow::{PaneInformation, TabInformation, UIItem, UIItemType};
use config::{ConfigHandle, TabBarColors};
use finl_unicode::grapheme_clusters::Graphemes;
use mlua::FromLua;
use mux::pane::CachePolicy;
use mux::Mux;
use std::path::Path;
use termwiz::cell::{unicode_column_width, Cell, CellAttributes};
use termwiz::color::{AnsiColor, ColorSpec};
use termwiz::escape::csi::Sgr;
use termwiz::escape::parser::Parser;
use termwiz::escape::{Action, ControlCode, CSI};
use termwiz::surface::SEQ_ZERO;
use termwiz_funcs::{format_as_escapes, FormatColor, FormatItem};
use url::Url;
use wezterm_term::{Line, Progress};
use window::{IntegratedTitleButton, IntegratedTitleButtonAlignment, IntegratedTitleButtonStyle};

#[derive(Clone, Debug, PartialEq)]
pub struct TabBarState {
    line: Line,
    items: Vec<TabEntry>,
    scroll_offset: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabBarItem {
    None,
    LeftStatus,
    RightStatus,
    Tab {
        tab_idx: usize,
        active: bool,
        renamed: bool,
    },
    NewTabButton,
    WindowButton(IntegratedTitleButton),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabEntry {
    pub item: TabBarItem,
    pub title: Line,
    pub x: usize,
    pub width: usize,
}

#[derive(Clone, Debug)]
struct TitleText {
    items: Vec<FormatItem>,
    len: usize,
}

fn call_format_tab_title(
    tab: &TabInformation,
    tab_info: &[TabInformation],
    pane_info: &[PaneInformation],
    config: &ConfigHandle,
    hover: bool,
    tab_max_width: usize,
) -> Option<TitleText> {
    match config::run_immediate_with_lua_config(|lua| {
        if let Some(lua) = lua {
            let tabs = lua.create_sequence_from(tab_info.iter().cloned())?;
            let panes = lua.create_sequence_from(pane_info.iter().cloned())?;

            let v = config::lua::emit_sync_callback(
                &*lua,
                (
                    "format-tab-title".to_string(),
                    (
                        tab.clone(),
                        tabs,
                        panes,
                        (**config).clone(),
                        hover,
                        tab_max_width,
                    ),
                ),
            )?;
            match &v {
                mlua::Value::Nil => Ok(None),
                mlua::Value::Table(_) => {
                    let items = <Vec<FormatItem>>::from_lua(v, &*lua)?;

                    let esc = format_as_escapes(items.clone())?;
                    let line = parse_status_text(&esc, CellAttributes::default());

                    Ok(Some(TitleText {
                        items,
                        len: line.len(),
                    }))
                }
                _ => {
                    let s = String::from_lua(v, &*lua)?;
                    let line = parse_status_text(&s, CellAttributes::default());
                    Ok(Some(TitleText {
                        len: line.len(),
                        items: vec![FormatItem::Text(s)],
                    }))
                }
            }
        } else {
            Ok(None)
        }
    }) {
        Ok(s) => s,
        Err(err) => {
            log::warn!("format-tab-title: {}", err);
            None
        }
    }
}

/// pct is a percentage in the range 0-100.
/// We want to map it to one of the nerdfonts:
///
/// * `md-checkbox_blank_circle_outline` (0xf0130) for an empty circle
/// * `md_circle_slice_1..=7` (0xf0a9e ..= 0xf0aa4) for a partly filled
///   circle
/// * `md_circle_slice_8` (0xf0aa5) for a filled circle
///
/// We use an empty circle for values close to 0%, a filled circle for values
/// close to 100%, and a partly filled circle for the rest (roughly evenly
/// distributed).
fn pct_to_glyph(pct: u8) -> char {
    match pct {
        0..=5 => '\u{f0130}',    // empty circle
        6..=18 => '\u{f0a9e}',   // centered at 12 (slightly smaller than 12.5)
        19..=31 => '\u{f0a9f}',  // centered at 25
        32..=43 => '\u{f0aa0}',  // centered at 37.5
        44..=56 => '\u{f0aa1}',  // half-filled circle, centered at 50
        57..=68 => '\u{f0aa2}',  // centered at 62.5
        69..=81 => '\u{f0aa3}',  // centered at 75
        82..=94 => '\u{f0aa4}',  // centered at 88 (slightly larger than 87.5)
        95..=100 => '\u{f0aa5}', // filled circle
        // Any other value is mapped to a filled circle.
        _ => '\u{f0aa5}',
    }
}

fn compute_tab_title(
    tab: &TabInformation,
    tab_info: &[TabInformation],
    pane_info: &[PaneInformation],
    config: &ConfigHandle,
    hover: bool,
    tab_max_width: usize,
) -> TitleText {
    let title = call_format_tab_title(tab, tab_info, pane_info, config, hover, tab_max_width);

    match title {
        Some(title) => title,
        None => {
            let mut items = vec![];
            let mut len = 0;

            if let Some(pane) = &tab.active_pane {
                let mut title = default_tab_title(tab, pane);

                let classic_spacing = if config.use_fancy_tab_bar { "" } else { " " };
                if config.show_tab_index_in_tab_bar {
                    let index = format!(
                        "{classic_spacing}{}: ",
                        tab.tab_index
                            + if config.tab_and_split_indices_are_zero_based {
                                0
                            } else {
                                1
                            }
                    );
                    len += unicode_column_width(&index, None);
                    items.push(FormatItem::Text(index));

                    title = format!("{}{classic_spacing}", title);
                }

                match pane.progress {
                    Progress::None => {}
                    Progress::Percentage(pct) | Progress::Error(pct) => {
                        let graphic = format!("{} ", pct_to_glyph(pct));
                        len += unicode_column_width(&graphic, None);
                        let color = if matches!(pane.progress, Progress::Percentage(_)) {
                            FormatItem::Foreground(FormatColor::AnsiColor(AnsiColor::Green))
                        } else {
                            FormatItem::Foreground(FormatColor::AnsiColor(AnsiColor::Red))
                        };
                        items.push(color);
                        items.push(FormatItem::Text(graphic));
                        items.push(FormatItem::Foreground(FormatColor::Default));
                    }
                    Progress::Indeterminate => {
                        // TODO: Decide what to do here to indicate this
                    }
                }

                // We have a preferred soft minimum on tab width to make it
                // easier to click on tab titles, but we'll still go below
                // this if there are too many tabs to fit the window at
                // this width.
                if !config.use_fancy_tab_bar {
                    while len + unicode_column_width(&title, None) < 5 {
                        title.push(' ');
                    }
                }

                len += unicode_column_width(&title, None);
                items.push(FormatItem::Text(title));
            } else {
                let title = " no pane ".to_string();
                len += unicode_column_width(&title, None);
                items.push(FormatItem::Text(title));
            };

            TitleText { len, items }
        }
    }
}

fn default_tab_title(tab: &TabInformation, pane: &PaneInformation) -> String {
    if !tab.tab_title.is_empty() {
        return tab.tab_title.clone();
    }

    if let Some(title) = tab_cwd_label(tab, pane) {
        return title;
    }

    sanitize_fallback_title(&pane.title)
}

fn tab_cwd_label(tab: &TabInformation, pane: &PaneInformation) -> Option<String> {
    let mux = Mux::try_get()?;

    if let Some(pane) = mux.get_pane(pane.pane_id) {
        if let Some(url) = pane.get_current_working_dir(CachePolicy::AllowStale) {
            if let Some(label) = cwd_label_from_url(&url) {
                return Some(label);
            }
        }
    }

    mux.get_tab(tab.tab_id)
        .and_then(|tab| tab.get_spawn_cwd())
        .and_then(cwd_label_from_path)
}

fn cwd_label_from_url(url: &Url) -> Option<String> {
    if url.scheme() != "file" {
        return None;
    }

    url.to_file_path()
        .ok()
        .and_then(cwd_label_from_path)
}

fn cwd_label_from_path<P: AsRef<Path>>(path: P) -> Option<String> {
    let path = path.as_ref();
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let display = path.display().to_string();
            if display.is_empty() {
                None
            } else {
                Some(display)
            }
        })
}

fn sanitize_fallback_title(title: &str) -> String {
    let path = Path::new(title);
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        return strip_shell_extension(name).unwrap_or(name).to_string();
    }

    strip_shell_extension(title).unwrap_or(title).to_string()
}

fn strip_shell_extension(title: &str) -> Option<&str> {
    let lower = title.to_ascii_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".com"] {
        if lower.ends_with(suffix) {
            return title.get(..title.len().saturating_sub(suffix.len()));
        }
    }
    None
}

fn is_tab_hover(mouse_x: Option<usize>, x: usize, tab_title_len: usize) -> bool {
    return mouse_x
        .map(|mouse_x| mouse_x >= x && mouse_x < x + tab_title_len)
        .unwrap_or(false);
}

impl TabBarState {
    pub fn default() -> Self {
        Self {
            line: Line::with_width(1, SEQ_ZERO),
            scroll_offset: 0,
            items: vec![TabEntry {
                item: TabBarItem::None,
                title: Line::from_text(" ", &CellAttributes::blank(), 1, None),
                x: 1,
                width: 1,
            }],
        }
    }

    pub fn line(&self) -> &Line {
        &self.line
    }

    pub fn items(&self) -> &[TabEntry] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.line.len()
    }

    pub fn scrollable_len(&self) -> usize {
        self.items
            .iter()
            .filter(|entry| {
                matches!(
                    entry.item,
                    TabBarItem::LeftStatus | TabBarItem::Tab { .. }
                )
            })
            .map(|entry| entry.x + entry.width)
            .max()
            .unwrap_or_else(|| self.line.len())
    }

    pub fn fixed_right_len(&self) -> usize {
        self.items
            .iter()
            .filter(|entry| {
                matches!(
                    entry.item,
                    TabBarItem::NewTabButton | TabBarItem::WindowButton(_)
                )
            })
            .map(|entry| entry.width)
            .sum()
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn tab_entry(&self, tab_idx: usize) -> Option<&TabEntry> {
        self.items
            .iter()
            .find(|entry| matches!(entry.item, TabBarItem::Tab { tab_idx: idx, .. } if idx == tab_idx))
    }

    fn integrated_title_buttons(
        mouse_x: Option<usize>,
        x: &mut usize,
        config: &ConfigHandle,
        items: &mut Vec<TabEntry>,
        line: &mut Line,
        colors: &TabBarColors,
    ) {
        let default_cell = if config.use_fancy_tab_bar {
            CellAttributes::default()
        } else {
            colors.new_tab().as_cell_attributes()
        };

        let default_cell_hover = if config.use_fancy_tab_bar {
            CellAttributes::default()
        } else {
            colors.new_tab_hover().as_cell_attributes()
        };

        let window_hide =
            parse_status_text(&config.tab_bar_style.window_hide, default_cell.clone());
        let window_hide_hover = parse_status_text(
            &config.tab_bar_style.window_hide_hover,
            default_cell_hover.clone(),
        );

        let window_maximize =
            parse_status_text(&config.tab_bar_style.window_maximize, default_cell.clone());
        let window_maximize_hover = parse_status_text(
            &config.tab_bar_style.window_maximize_hover,
            default_cell_hover.clone(),
        );

        let window_close =
            parse_status_text(&config.tab_bar_style.window_close, default_cell.clone());
        let window_close_hover = parse_status_text(
            &config.tab_bar_style.window_close_hover,
            default_cell_hover.clone(),
        );

        for button in &config.integrated_title_buttons {
            use IntegratedTitleButton as Button;
            let title = match button {
                Button::Hide => {
                    let hover = is_tab_hover(mouse_x, *x, window_hide_hover.len());

                    if hover {
                        &window_hide_hover
                    } else {
                        &window_hide
                    }
                }
                Button::Maximize => {
                    let hover = is_tab_hover(mouse_x, *x, window_maximize_hover.len());

                    if hover {
                        &window_maximize_hover
                    } else {
                        &window_maximize
                    }
                }
                Button::Close => {
                    let hover = is_tab_hover(mouse_x, *x, window_close_hover.len());

                    if hover {
                        &window_close_hover
                    } else {
                        &window_close
                    }
                }
            };

            line.append_line(title.to_owned(), SEQ_ZERO);

            let width = title.len();
            items.push(TabEntry {
                item: TabBarItem::WindowButton(*button),
                title: title.to_owned(),
                x: *x,
                width,
            });

            *x += width;
        }
    }

    /// Build a new tab bar from the current state
    /// mouse_x is some if the mouse is on the same row as the tab bar.
    /// title_width is the total number of cell columns in the window.
    /// window allows access to the tabs associated with the window.
    pub fn new(
        title_width: usize,
        mouse_x: Option<usize>,
        tab_info: &[TabInformation],
        pane_info: &[PaneInformation],
        colors: Option<&TabBarColors>,
        config: &ConfigHandle,
        left_status: &str,
        right_status: &str,
        scroll_offset: usize,
    ) -> Self {
        let colors = colors.cloned().unwrap_or_else(TabBarColors::default);

        let active_cell_attrs = colors.active_tab().as_cell_attributes();
        let inactive_hover_attrs = colors.inactive_tab_hover().as_cell_attributes();
        let inactive_cell_attrs = colors.inactive_tab().as_cell_attributes();
        let new_tab_hover_attrs = colors.new_tab_hover().as_cell_attributes();
        let new_tab_attrs = colors.new_tab().as_cell_attributes();

        let new_tab = parse_status_text(
            &config.tab_bar_style.new_tab,
            if config.use_fancy_tab_bar {
                CellAttributes::default()
            } else {
                new_tab_attrs.clone()
            },
        );
        let new_tab_hover = parse_status_text(
            &config.tab_bar_style.new_tab_hover,
            if config.use_fancy_tab_bar {
                CellAttributes::default()
            } else {
                new_tab_hover_attrs.clone()
            },
        );

        let use_integrated_title_buttons = config
            .window_decorations
            .contains(window::WindowDecorations::INTEGRATED_BUTTONS);

        // We ultimately want to produce a line looking like this:
        // ` | tab1-title x | tab2-title x |  +      . - X `
        // Where the `+` sign will spawn a new tab (or show a context
        // menu with tab creation options) and the other three chars
        // are symbols representing minimize, maximize and close.

        let mut active_tab_no = 0;

        let tab_titles: Vec<TitleText> = if config.show_tabs_in_tab_bar {
            tab_info
                .iter()
                .map(|tab| {
                    if tab.is_active {
                        active_tab_no = tab.tab_index;
                    }
                    compute_tab_title(
                        tab,
                        tab_info,
                        pane_info,
                        config,
                        false,
                        config.tab_max_width,
                    )
                })
                .collect()
        } else {
            vec![]
        };
        let mut line = Line::with_width(0, SEQ_ZERO);

        let mut x = 0;
        let mut items = vec![];

        let black_cell = Cell::blank_with_attrs(
            CellAttributes::default()
                .set_background(ColorSpec::TrueColor(*colors.background()))
                .clone(),
        );

        if use_integrated_title_buttons
            && config.integrated_title_button_style == IntegratedTitleButtonStyle::MacOsNative
            && config.use_fancy_tab_bar == false
            && config.tab_bar_at_bottom == false
        {
            for _ in 0..10 as usize {
                line.insert_cell(0, black_cell.clone(), title_width, SEQ_ZERO);
                x += 1;
            }
        }

        if use_integrated_title_buttons
            && config.integrated_title_button_style != IntegratedTitleButtonStyle::MacOsNative
            && config.integrated_title_button_alignment == IntegratedTitleButtonAlignment::Left
        {
            Self::integrated_title_buttons(mouse_x, &mut x, config, &mut items, &mut line, &colors);
        }

        let left_status_line = parse_status_text(left_status, black_cell.attrs().clone());
        if left_status_line.len() > 0 {
            items.push(TabEntry {
                item: TabBarItem::LeftStatus,
                title: left_status_line.clone(),
                x,
                width: left_status_line.len(),
            });
            x += left_status_line.len();
            line.append_line(left_status_line, SEQ_ZERO);
        }

        for (tab_idx, tab_title) in tab_titles.iter().enumerate() {
            let renamed = !tab_info[tab_idx].tab_title.is_empty();
            let tab_title_len = tab_title.len;
            let active = tab_idx == active_tab_no;
            let hover = !active && is_tab_hover(mouse_x, x, tab_title_len);

            // Recompute the title so that it factors in both the hover state
            // and the adjusted maximum tab width based on available space.
            let tab_title = compute_tab_title(
                &tab_info[tab_idx],
                tab_info,
                pane_info,
                config,
                hover,
                tab_title_len,
            );

            let cell_attrs = if active {
                &active_cell_attrs
            } else if hover {
                &inactive_hover_attrs
            } else {
                &inactive_cell_attrs
            };

            let tab_start_idx = x;

            let esc = format_as_escapes(tab_title.items.clone()).expect("already parsed ok above");
            let mut tab_line = parse_status_text(
                &esc,
                if config.use_fancy_tab_bar {
                    CellAttributes::default()
                } else {
                    cell_attrs.clone()
                },
            );

            let title = tab_line.clone();
            let width = tab_line.len();

            items.push(TabEntry {
                item: TabBarItem::Tab {
                    tab_idx,
                    active,
                    renamed,
                },
                title,
                x: tab_start_idx,
                width,
            });

            line.append_line(tab_line, SEQ_ZERO);
            x += width;
        }

        // New tab button
        if config.show_new_tab_button_in_tab_bar {
            let hover = is_tab_hover(mouse_x, x, new_tab_hover.len());

            let new_tab_button = if hover { &new_tab_hover } else { &new_tab };

            let button_start = x;
            let width = new_tab_button.len();

            line.append_line(new_tab_button.clone(), SEQ_ZERO);

            items.push(TabEntry {
                item: TabBarItem::NewTabButton,
                title: new_tab_button.clone(),
                x: button_start,
                width,
            });

            x += width;
        }

        // Reserve place for integrated title buttons
        let title_width = if use_integrated_title_buttons
            && config.integrated_title_button_style != IntegratedTitleButtonStyle::MacOsNative
            && config.integrated_title_button_alignment == IntegratedTitleButtonAlignment::Right
        {
            let window_hide =
                parse_status_text(&config.tab_bar_style.window_hide, CellAttributes::default());
            let window_hide_hover = parse_status_text(
                &config.tab_bar_style.window_hide_hover,
                CellAttributes::default(),
            );

            let window_maximize = parse_status_text(
                &config.tab_bar_style.window_maximize,
                CellAttributes::default(),
            );
            let window_maximize_hover = parse_status_text(
                &config.tab_bar_style.window_maximize_hover,
                CellAttributes::default(),
            );
            let window_close = parse_status_text(
                &config.tab_bar_style.window_close,
                CellAttributes::default(),
            );
            let window_close_hover = parse_status_text(
                &config.tab_bar_style.window_close_hover,
                CellAttributes::default(),
            );

            let hide_len = window_hide.len().max(window_hide_hover.len());
            let maximize_len = window_maximize.len().max(window_maximize_hover.len());
            let close_len = window_close.len().max(window_close_hover.len());

            let mut width_to_reserve = 0;
            for button in &config.integrated_title_buttons {
                use IntegratedTitleButton as Button;
                let button_len = match button {
                    Button::Hide => hide_len,
                    Button::Maximize => maximize_len,
                    Button::Close => close_len,
                };
                width_to_reserve += button_len;
            }

            title_width.saturating_sub(width_to_reserve)
        } else {
            title_width
        };

        let status_space_available = title_width.saturating_sub(x);

        let mut right_status_line = parse_status_text(right_status, black_cell.attrs().clone());
        items.push(TabEntry {
            item: TabBarItem::RightStatus,
            title: right_status_line.clone(),
            x,
            width: status_space_available,
        });

        while right_status_line.len() > status_space_available {
            right_status_line.remove_cell(0, SEQ_ZERO);
        }

        line.append_line(right_status_line, SEQ_ZERO);
        while line.len() < title_width {
            line.insert_cell(x, black_cell.clone(), title_width, SEQ_ZERO);
        }

        if use_integrated_title_buttons
            && config.integrated_title_button_style != IntegratedTitleButtonStyle::MacOsNative
            && config.integrated_title_button_alignment == IntegratedTitleButtonAlignment::Right
        {
            x = title_width;
            Self::integrated_title_buttons(mouse_x, &mut x, config, &mut items, &mut line, &colors);
        }

        Self {
            line,
            items,
            scroll_offset,
        }
    }

    pub fn compute_ui_items(
        &self,
        y: usize,
        cell_height: usize,
        cell_width: usize,
        visible_width: usize,
    ) -> Vec<UIItem> {
        let mut items = vec![];

        for entry in self.items.iter() {
            let entry_start = entry.x.saturating_sub(self.scroll_offset);
            let hidden = self.scroll_offset.saturating_sub(entry.x);
            let visible_cells = entry.width.saturating_sub(hidden);
            if visible_cells == 0 || entry_start >= visible_width {
                continue;
            }
            let visible_cells = visible_cells.min(visible_width.saturating_sub(entry_start));
            items.push(UIItem {
                x: entry_start * cell_width,
                width: visible_cells * cell_width,
                y,
                height: cell_height,
                item_type: UIItemType::TabBar(entry.item),
            });
        }

        items
    }
}

pub fn parse_status_text(text: &str, default_cell: CellAttributes) -> Line {
    let mut pen = default_cell.clone();
    let mut cells = vec![];
    let mut ignoring = false;
    let mut print_buffer = String::new();

    fn flush_print(buf: &mut String, cells: &mut Vec<Cell>, pen: &CellAttributes) {
        for g in Graphemes::new(buf.as_str()) {
            let cell = Cell::new_grapheme(g, pen.clone(), None);
            let width = cell.width();
            cells.push(cell);
            for _ in 1..width {
                // Line/Screen expect double wide graphemes to be followed by a blank in
                // the next column position, otherwise we'll render incorrectly
                cells.push(Cell::blank_with_attrs(pen.clone()));
            }
        }
        buf.clear();
    }

    let mut parser = Parser::new();
    parser.parse(text.as_bytes(), |action| {
        if ignoring {
            return;
        }
        match action {
            Action::Print(c) => print_buffer.push(c),
            Action::PrintString(s) => print_buffer.push_str(&s),
            Action::Control(c) => {
                flush_print(&mut print_buffer, &mut cells, &pen);
                match c {
                    ControlCode::CarriageReturn | ControlCode::LineFeed => {
                        ignoring = true;
                    }
                    _ => {}
                }
            }
            Action::CSI(csi) => {
                flush_print(&mut print_buffer, &mut cells, &pen);
                match csi {
                    CSI::Sgr(sgr) => match sgr {
                        Sgr::Reset => pen = default_cell.clone(),
                        Sgr::Intensity(i) => {
                            pen.set_intensity(i);
                        }
                        Sgr::Underline(u) => {
                            pen.set_underline(u);
                        }
                        Sgr::Overline(o) => {
                            pen.set_overline(o);
                        }
                        Sgr::VerticalAlign(o) => {
                            pen.set_vertical_align(o);
                        }
                        Sgr::Blink(b) => {
                            pen.set_blink(b);
                        }
                        Sgr::Italic(i) => {
                            pen.set_italic(i);
                        }
                        Sgr::Inverse(inverse) => {
                            pen.set_reverse(inverse);
                        }
                        Sgr::Invisible(invis) => {
                            pen.set_invisible(invis);
                        }
                        Sgr::StrikeThrough(strike) => {
                            pen.set_strikethrough(strike);
                        }
                        Sgr::Foreground(col) => {
                            if let ColorSpec::Default = col {
                                pen.set_foreground(default_cell.foreground());
                            } else {
                                pen.set_foreground(col);
                            }
                        }
                        Sgr::Background(col) => {
                            if let ColorSpec::Default = col {
                                pen.set_background(default_cell.background());
                            } else {
                                pen.set_background(col);
                            }
                        }
                        Sgr::UnderlineColor(col) => {
                            pen.set_underline_color(col);
                        }
                        Sgr::Font(_) => {}
                    },
                    _ => {}
                }
            }
            Action::OperatingSystemCommand(_)
            | Action::DeviceControl(_)
            | Action::Esc(_)
            | Action::KittyImage(_)
            | Action::XtGetTcap(_)
            | Action::Sixel(_) => {
                flush_print(&mut print_buffer, &mut cells, &pen);
            }
        }
    });
    flush_print(&mut print_buffer, &mut cells, &pen);
    Line::from_cells(cells, SEQ_ZERO)
}
