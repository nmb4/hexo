//! Exo for HEY's screen: the Classic Exo chrome (tab strip, rail, thread list, contact pane)
//! wrapped around HEY's mailboxes, screener, labels, search and compose.

use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;

use egui::{
    Align2, Color32, CursorIcon, FontFamily, FontId, Pos2, Rect, RichText, ScrollArea, Sense,
    Stroke, Ui, UiBuilder, Vec2, pos2, vec2,
};
use exo_commands::{Command, Commands, Region, Toast};
use exo_store::{
    BoxInfo, Identity, Label, ScreenerItem, SearchHit, Thread, ThreadRow,
};
use exo_ui::{Family, Font, TextStyle};
use hey_sdk::{Config as HeyConfig, services::BoxKind};
use jiff::ToSpan as _;

use crate::{
    backend::{Command as BackendCommand, Event as BackendEvent},
    body::BodyRenderer,
    theme::Palette,
};

const BOX_ORDER: &[&str] = &["imbox", "feedbox", "laterbox", "asidebox", "trailbox", "bubblebox"];
const MAIL: &[Region] = &[Region::ThreadList, Region::ThreadMessages];
const COMPOSE_REGIONS: &[Region] = &[Region::ComposeForm];

// The original app's metrics, in logical pixels.
const TAB_BAR: f32 = 41.0;
const RAIL: f32 = 40.0;
const HEADER: f32 = 76.0;
const ROW: f32 = 36.0;
const SEPARATOR: f32 = 48.0;
const LIST_PAD_TOP: f32 = 2.0;
const VIEW_PAD_X: f32 = 6.0;

// The original app's type, exactly.
const TAB: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
const COUNT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 24.0);
const ROW_TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const ROW_BOLD: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
const ROW_DATE: TextStyle = ROW_TEXT.letter_spacing(0.017 * 12.0);
const NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
const BOLD: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
const DATA: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const SECTION: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 10.0, 16.0);
const SUBJECT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 21.0, 32.0);
const THREAD_FROM: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 16.0);
const THREAD_DATE: TextStyle = ROW_DATE;
const TITLE: TextStyle = TextStyle::new(Font::new(Family::System, 400), 12.8, 22.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Mail,
    Screener,
    Search,
}

#[derive(Clone, Debug, Default)]
struct ComposeDraft {
    to: String,
    subject: String,
    body: String,
}

#[derive(Clone, Debug)]
enum Action {
    Next,
    Prev,
    Open,
    Back,
    SetAside,
    Reply,
    Compose,
    Trash,
    Spam,
    Bubble,
    Refresh,
    Search,
    Screener,
    Go(String),
    Label(i64),
    ToggleTheme,
    SignOut,
    Send,
    SaveDraft,
    Cancel,
}

#[derive(Clone, Debug)]
enum TabTarget {
    Box(String),
    Screener,
}

/// The app. All network work lives in [`crate::backend`]; this struct only holds snapshots.
pub struct ExoApp {
    p: Palette,
    commands: Commands<Action>,
    tx: mpsc::UnboundedSender<BackendCommand>,
    rx: mpsc::UnboundedReceiver<BackendEvent>,

    signed_in: bool,
    starting: bool,
    identity: Identity,
    boxes: Vec<BoxInfo>,
    labels: Vec<Label>,
    rows: HashMap<String, Vec<ThreadRow>>,
    selected_box: String,
    selected: Option<usize>,
    thread: Option<Thread>,
    screener: Vec<ScreenerItem>,
    hits: Option<Vec<SearchHit>>,
    view: View,

    search_text: String,
    search_focus: bool,
    compose: Option<ComposeDraft>,
    compose_focus: bool,
    reply_open: bool,
    reply_text: String,

    drawer_open: bool,
    settings_open: bool,

    body: BodyRenderer,
    status: String,
    error: Option<String>,
}

impl ExoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Exo's fonts and icons are optional polish: on Linux/Windows the system fallback is
        // used when a font file isn't available.
        let _ = exo_ui::install(&cc.egui_ctx);
        let palette = Palette::light();
        palette.install(&cc.egui_ctx);

        let (tx_cmd, rx_cmd) = mpsc::unbounded_channel();
        let (tx_event, rx_event) = mpsc::unbounded_channel();
        let config = HeyConfig::default();
        let credentials = exo_auth::CredentialStore::new();
        let provider = std::sync::Arc::new(exo_auth::TokenProvider::new(
            credentials.clone(),
            config.clone(),
        ));
        let signed_in = credentials.load().ok().flatten().is_some();
        std::thread::Builder::new()
            .name("exo-hey-backend".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                runtime.block_on(crate::backend::run(
                    config, credentials, provider, rx_cmd, tx_event,
                ));
            })
            .expect("spawn backend");

        if signed_in {
            let _ = tx_cmd.send(BackendCommand::Bootstrap);
        }

        Self {
            p: palette,
            commands: Commands::new(),
            tx: tx_cmd,
            rx: rx_event,
            signed_in,
            starting: signed_in,
            identity: Identity::default(),
            boxes: Vec::new(),
            labels: Vec::new(),
            rows: HashMap::new(),
            selected_box: "imbox".into(),
            selected: None,
            thread: None,
            screener: Vec::new(),
            hits: None,
            view: View::Mail,
            search_text: String::new(),
            search_focus: false,
            compose: None,
            compose_focus: false,
            reply_open: false,
            reply_text: String::new(),
            drawer_open: false,
            settings_open: false,
            body: BodyRenderer::new(),
            status: "Ready".into(),
            error: None,
        }
    }

    fn send(&mut self, command: BackendCommand) {
        if self.tx.send(command).is_err() {
            self.error = Some("The background worker stopped".into());
        }
    }

    fn current_rows(&self) -> &[ThreadRow] {
        self.rows.get(&self.selected_box).map(Vec::as_slice).unwrap_or_default()
    }

    fn current_posting(&self) -> Option<&ThreadRow> {
        self.selected.and_then(|i| self.current_rows().get(i))
    }

    fn box_name(&self, kind: &str) -> String {
        self.boxes
            .iter()
            .find(|b| b.kind == kind)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| match kind {
                "imbox" => "The Imbox".into(),
                "feedbox" => "The Feed".into(),
                "laterbox" => "Reply Later".into(),
                "asidebox" => "Set Aside".into(),
                "trailbox" => "Paper Trail".into(),
                "bubblebox" => "Bubble Up".into(),
                other => other.to_string(),
            })
    }

    fn select_box(&mut self, kind: &str) {
        self.selected_box = kind.to_string();
        self.view = View::Mail;
        self.selected = None;
        self.thread = None;
        self.reply_open = false;
        self.drawer_open = false;
        self.send(BackendCommand::LoadBox(kind.to_string()));
    }

    fn open_screener(&mut self) {
        self.view = View::Screener;
        self.thread = None;
        self.selected = None;
        self.drawer_open = false;
    }

    fn open_search(&mut self) {
        self.view = View::Search;
        self.thread = None;
        self.search_focus = true;
        self.hits = None;
        self.drawer_open = false;
    }

    fn close_thread(&mut self) {
        self.thread = None;
        self.reply_open = false;
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Next => {
                let len = self.current_rows().len();
                if len > 0 {
                    let next = self.selected.map_or(0, |i| (i + 1).min(len - 1));
                    self.selected = Some(next);
                    if self.thread.is_some() {
                        self.open_selected();
                    }
                }
            }
            Action::Prev => {
                let len = self.current_rows().len();
                if len > 0 {
                    let prev = self.selected.map_or(0, |i| i.saturating_sub(1));
                    self.selected = Some(prev);
                    if self.thread.is_some() {
                        self.open_selected();
                    }
                }
            }
            Action::Open => self.open_selected(),
            Action::Back => {
                if self.drawer_open {
                    self.drawer_open = false;
                } else if self.thread.is_some() {
                    self.close_thread();
                } else if self.view != View::Mail {
                    self.view = View::Mail;
                }
            }
            Action::SetAside => {
                if let Some(row) = self.current_posting() {
                    self.send(BackendCommand::Move(BoxKind::SetAside, vec![row.posting_id]));
                    self.selected = None;
                    self.thread = None;
                }
            }
            Action::Trash => {
                if let Some(row) = self.current_posting() {
                    self.send(BackendCommand::Trash(vec![row.posting_id]));
                    self.selected = None;
                    self.thread = None;
                }
            }
            Action::Spam => {
                if let Some(row) = self.current_posting() {
                    self.send(BackendCommand::Spam(vec![row.posting_id]));
                    self.selected = None;
                    self.thread = None;
                }
            }
            Action::Bubble => {
                if let Some(row) = self.current_posting() {
                    self.send(BackendCommand::Bubble(vec![row.posting_id]));
                }
            }
            Action::Reply => {
                if self.thread.is_some() {
                    self.reply_open = true;
                    self.reply_text.clear();
                }
            }
            Action::Compose => {
                self.compose = Some(ComposeDraft::default());
                self.compose_focus = true;
                self.drawer_open = false;
            }
            Action::Refresh => {
                self.starting = true;
                self.send(BackendCommand::Refresh);
            }
            Action::Search => self.open_search(),
            Action::Screener => self.open_screener(),
            Action::Go(kind) => self.select_box(&kind),
            Action::Label(id) => {
                if let Some(row) = self.current_posting() {
                    if row.label_ids.contains(&id) {
                        self.send(BackendCommand::LabelRemove(row.posting_id, id));
                    } else {
                        self.send(BackendCommand::LabelAdd(row.posting_id, id));
                    }
                }
            }
            Action::ToggleTheme => {
                self.p = if self.p.dark { Palette::light() } else { Palette::dark() };
            }
            Action::SignOut => {
                self.signed_in = false;
                self.starting = false;
                self.thread = None;
                self.send(BackendCommand::SignOut);
            }
            Action::Send => self.send_compose(false),
            Action::SaveDraft => self.send_compose(true),
            Action::Cancel => {
                self.compose = None;
                self.reply_open = false;
                self.drawer_open = false;
                self.settings_open = false;
                self.commands.close_palette();
            }
        }
    }

    fn open_selected(&mut self) {
        if let Some(row) = self.current_posting().cloned() {
            if row.topic_id != 0 {
                self.thread = None;
                self.view = View::Mail;
                self.send(BackendCommand::LoadThread(row.topic_id));
                self.status = format!("Opening {}", row.subject);
            } else {
                self.status = "This bundle has no single thread to open".into();
            }
        }
    }

    fn open_topic(&mut self, topic_id: i64) {
        if topic_id == 0 {
            return;
        }
        self.view = View::Mail;
        self.thread = None;
        self.send(BackendCommand::LoadThread(topic_id));
    }

    fn send_compose(&mut self, draft: bool) {
        let Some(compose) = self.compose.clone() else { return };
        let to = split_addresses(&compose.to);
        if to.is_empty() {
            self.error = Some("Add at least one recipient".into());
            return;
        }
        let html = exo_mail::text_to_html(&compose.body);
        if draft {
            self.send(BackendCommand::SaveDraft { to, subject: compose.subject, body: html });
        } else {
            self.send(BackendCommand::Send { to, subject: compose.subject, body: html });
        }
        self.compose = None;
    }

    fn pump_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                BackendEvent::SignedIn { identity } => {
                    self.signed_in = true;
                    self.identity = identity;
                }
                BackendEvent::SignedOut => {
                    self.signed_in = false;
                    self.starting = false;
                    self.thread = None;
                    self.rows.clear();
                }
                BackendEvent::Bootstrapped {
                    identity,
                    boxes,
                    labels,
                    screener,
                    rows,
                } => {
                    self.signed_in = true;
                    self.starting = false;
                    self.identity = identity;
                    self.boxes = boxes;
                    self.labels = labels;
                    self.screener = screener;
                    self.rows = rows;
                    self.status = "Synced".into();
                    self.ensure_selection();
                }
                BackendEvent::BoxLoaded { kind, rows } => {
                    self.rows.insert(kind.clone(), rows);
                    if kind == self.selected_box {
                        self.ensure_selection();
                    }
                }
                BackendEvent::BoxesUpdated { rows } => {
                    self.rows = rows;
                    self.ensure_selection();
                }
                BackendEvent::ThreadLoaded { thread } => {
                    self.thread = Some(thread);
                    self.reply_open = false;
                    self.status = "Synced".into();
                }
                BackendEvent::ScreenerLoaded { items } => self.screener = items,
                BackendEvent::SearchDone { hits } => {
                    self.hits = Some(hits);
                    self.view = View::Search;
                }
                BackendEvent::Done { message } => {
                    self.status = message.clone();
                    self.commands.toast(Toast::new(message));
                }
                BackendEvent::Error { message } => {
                    self.error = Some(message.clone());
                    self.status = "Error".into();
                }
            }
        }
    }

    fn ensure_selection(&mut self) {
        let len = self.current_rows().len();
        match self.selected {
            _ if len == 0 => self.selected = None,
            Some(index) if index < len => {}
            _ => self.selected = Some(0),
        }
    }

    fn register_commands(&mut self) {
        let c = &mut self.commands;
        c.add(Command::shortcut("NEXT", Action::Next).keys(["j", "down"]).regions(MAIL));
        c.add(Command::shortcut("PREV", Action::Prev).keys(["k", "up"]).regions(MAIL));
        c.add(Command::new("OPEN", "Open thread", Action::Open).key("enter").regions(MAIL));
        c.add(Command::new("BACK", "Back", Action::Back).key("esc").regions(&[Region::ThreadMessages]));
        c.add(Command::new("SET_ASIDE", "Set Aside", Action::SetAside).keys(["a", "e"]).regions(MAIL).icon("data/done"));
        c.add(Command::new("REPLY", "Reply", Action::Reply).key("r").regions(MAIL));
        c.add(Command::new("COMPOSE", "Compose", Action::Compose).key("c").regions(MAIL));
        c.add(Command::new("TRASH", "Move to Trash", Action::Trash).keys(["t", "#"]).regions(MAIL));
        c.add(Command::new("SPAM", "Mark as spam", Action::Spam).key("!").regions(MAIL));
        c.add(Command::new("BUBBLE", "Bubble up", Action::Bubble).key("b").regions(MAIL));
        c.add(Command::new("REFRESH", "Refresh", Action::Refresh).keys(["mod+r", "g r"]).regions(MAIL));
        c.add(Command::new("SEARCH", "Search", Action::Search).keys(["s", "/"]).regions(MAIL));
        c.add(Command::new("SCREENER", "The Screener", Action::Screener).keys(["mod+s", "g s"]).regions(MAIL));
        c.add(Command::new("INBOX", "go to The Imbox", Action::Go("imbox".into())).keys(["i", "g i"]).regions(MAIL));
        c.add(Command::new("FEED", "go to The Feed", Action::Go("feedbox".into())).keys(["d", "g f"]).regions(MAIL));
        c.add(Command::new("TRAIL", "go to Paper Trail", Action::Go("trailbox".into())).keys(["p", "g t"]).regions(MAIL));
        c.add(Command::new("LATER", "go to Reply Later", Action::Go("laterbox".into())).keys(["l", "g l"]).regions(MAIL));
        c.add(Command::new("ASIDE", "go to Set Aside", Action::Go("asidebox".into())).keys(["g a"]).regions(MAIL));
        c.add(Command::new("THEME", "Toggle light/dark", Action::ToggleTheme).key("mod+d").regions(MAIL));
        c.add(Command::new("SIGN_OUT", "Sign out", Action::SignOut).regions(MAIL));
        for label in &self.labels {
            c.add(Command::new(
                format!("LABEL_{}", label.id),
                format!("Label: {}", label.name),
                Action::Label(label.id),
            )
            .regions(MAIL)
            .icon("data/label"));
        }
        c.add(Command::new("SEND", "Send message", Action::Send).key("mod+enter").regions(COMPOSE_REGIONS));
        c.add(Command::new("DRAFT", "Save draft", Action::SaveDraft).regions(COMPOSE_REGIONS));
        c.add(Command::new("CANCEL", "Cancel", Action::Cancel).key("esc").regions(COMPOSE_REGIONS));
    }

    // ---------------------------------------------------------------------------------------
    // The screen: tab strip, rail, content and contact pane.
    // ---------------------------------------------------------------------------------------

    fn paint_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.signed_in {
            egui::CentralPanel::default_margins().show(ui, |ui| self.sign_in_ui(ui));
            return;
        }

        let window = ui.max_rect();
        ui.painter().rect_filled(window, 0.0, self.p.window);

        let strip = Rect::from_min_size(window.min, vec2(window.width(), TAB_BAR));
        self.tab_strip(ui, strip);

        let page = Rect::from_min_max(pos2(window.left(), strip.bottom()), window.max);
        let right_width = exo_ui::right_pane_width(window.width());
        let rail = Rect::from_min_size(page.min, vec2(RAIL, page.height()));
        let right = Rect::from_min_max(pos2(page.right() - right_width, page.top()), page.max);
        let content = Rect::from_min_max(
            pos2(rail.right(), page.top()),
            pos2(right.left(), page.bottom()),
        );

        self.rail_ui(ui, rail);
        self.content_ui(ui, content);
        self.contact_ui(ui, right);

        if self.drawer_open {
            self.drawer_ui(ui, content);
        }
        if self.settings_open {
            self.settings_ui(ui, right);
        }

        if let Some(compose) = self.compose.clone() {
            egui::Window::new("New message")
                .title_bar(false)
                .default_width(660.0)
                .default_height(480.0)
                .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
                .resizable(true)
                .frame(
                    egui::Frame::window(ui.style())
                        .fill(self.p.panel)
                        .corner_radius(egui::CornerRadius::same(8))
                        .stroke(Stroke::new(1.0, self.p.line)),
                )
                .show(&ctx, |ui| self.compose_ui(ui, compose));
        }
    }

    fn window_title(&self) -> String {
        match self.view {
            View::Mail => format!(
                "{} ({}) - Exo",
                self.box_name(&self.selected_box),
                self.current_rows().len()
            ),
            View::Screener => format!("The Screener ({}) - Exo", self.screener.len()),
            View::Search => "Search - Exo".into(),
        }
    }

    fn tab_strip(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.p;
        ui.painter().rect_filled(rect, 0.0, p.chrome);
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, p.line),
        );
        // A 75px margin leaves room for the native traffic lights.
        let drag_left = Rect::from_min_max(rect.min, pos2(rect.left() + 75.0, rect.bottom()));
        let drag_right = Rect::from_min_max(pos2(rect.right() - 75.0, rect.top()), rect.max);
        for (index, area) in [drag_left, drag_right].into_iter().enumerate() {
            let response = ui
                .interact(area, egui::Id::new(("strip-drag", index)), Sense::click_and_drag())
                .on_hover_cursor(CursorIcon::Default);
            if response.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if response.double_clicked() {
                let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
        }

        let pill = Rect::from_min_size(
            pos2(rect.left() + 84.0, rect.top() + 8.0),
            vec2((rect.width() - 192.0).max(160.0), 24.0),
        );
        let response = ui.interact(pill, egui::Id::new("tab-pill"), Sense::click());
        if response.double_clicked() {
            let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
        let border = if p.dark {
            Color32::from_white_alpha(28)
        } else {
            Color32::from_black_alpha(28)
        };
        ui.painter().rect_stroke(
            pill,
            egui::CornerRadius::same(4),
            Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );

        let title = self.window_title();
        let content = pill.shrink2(vec2(13.8, 1.0));
        let room = (content.width() - 10.0).max(0.0);
        let text = TITLE.single_line(
            ui.painter(),
            &title,
            p.ink.gamma_multiply(0.82),
            Some(room),
        );
        let lead = 10.0;
        let line = pos2(
            content.center().x - (lead + text.size().x) / 2.0,
            content.top(),
        );
        let baseline = content.top() + TITLE.baseline(ui.ctx());
        let dot = Rect::from_min_size(pos2(line.x, baseline - 8.0), Vec2::splat(8.0));
        ui.painter().rect_filled(
            dot,
            egui::CornerRadius::same(2),
            p.account_dot.gamma_multiply(if ui.ctx().input(|i| i.focused) { 0.9 } else { 0.45 }),
        );
        let painter = ui.painter().with_clip_rect(content);
        exo_ui::paint_text(
            &painter,
            pos2(line.x + lead, line.y),
            text,
            &TITLE,
        );

        let add = Rect::from_min_size(pos2(pill.right() + 8.0, pill.top()), Vec2::splat(24.0));
        let add_response = ui
            .interact(add, egui::Id::new("tab-add"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Compose");
        let add_color = if add_response.hovered() { p.icon_hover } else { p.muted };
        exo_ui::paint_icon(
            ui,
            "shell/tab-add",
            Rect::from_center_size(add.center(), Vec2::splat(14.0)),
            add_color,
        );
        if add_response.clicked() {
            self.apply(Action::Compose);
        }
    }

    fn rail_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.p;
        let mut ui = ui.new_child(UiBuilder::new().max_rect(rect));
        ui.set_clip_rect(rect);
        ui.painter().rect_filled(rect, 0.0, p.chrome);
        ui.painter().vline(
            rect.right() - 0.5,
            rect.y_range(),
            Stroke::new(1.0, p.line),
        );

        // The Mail/Screener switcher, drawn exactly as Exo's rail: a 30×60 well with a 24px
        // raised knob under the active app.
        let switcher = Rect::from_min_size(
            pos2(rect.left() + 5.0, rect.top() + 27.0),
            vec2(30.0, 60.0),
        );
        let well = if p.dark {
            Color32::from_rgba_unmultiplied(24, 25, 26, 128)
        } else {
            Color32::from_rgb(0xf2, 0xf4, 0xfa)
        };
        ui.painter().rect_filled(switcher, egui::CornerRadius::same(15), well);

        let screener = self.view == View::Screener;
        let knob = Rect::from_min_size(
            switcher.min + vec2(3.0, 3.0 + if screener { 29.0 } else { 0.0 }),
            Vec2::splat(24.0),
        );
        exo_ui::BoxShadow::new([0.0, 0.0], 0.5, 0.0, Color32::from_black_alpha(26))
            .paint(ui.painter(), knob, egui::CornerRadius::same(12));
        exo_ui::BoxShadow::new([0.0, 0.0], 6.0, 0.0, Color32::from_black_alpha(38))
            .paint(ui.painter(), knob, egui::CornerRadius::same(12));
        ui.painter().rect_filled(
            knob,
            egui::CornerRadius::same(12),
            if p.dark {
                Color32::from_rgb(0x36, 0x3a, 0x40)
            } else {
                Color32::WHITE
            },
        );

        let buttons = [
            (0.0, "legacy/DEPRECATED_emailApp", "Mail", false),
            (1.0, "legacy/DEPRECATED_calendarApp", "Calendar", true),
        ];
        let mut action = None;
        let mut calendar = false;
        for (row, icon, tooltip, is_calendar) in buttons {
            let hit = Rect::from_min_size(
                switcher.min + vec2(3.0, 4.0 + 28.0 * row),
                Vec2::splat(24.0),
            );
            let active = if is_calendar { false } else { !screener };
            let response = ui
                .interact(hit, egui::Id::new(("rail", icon)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(tooltip);
            let color = if active {
                if p.dark { Color32::from_rgb(0xc3, 0xc4, 0xc6) } else { Color32::from_rgb(0x4c, 0x4c, 0x4c) }
            } else if response.hovered() {
                if p.dark { Color32::from_rgb(0x79, 0x7a, 0x7c) } else { Color32::from_rgb(0x79, 0x7a, 0x7c) }
            } else if p.dark {
                Color32::from_rgb(0x5d, 0x5e, 0x5f)
            } else {
                Color32::from_rgb(0x91, 0x92, 0x95)
            };
            let glyph = if is_calendar {
                Vec2::splat(14.0)
            } else {
                vec2(14.0, 12.0)
            };
            exo_ui::paint_icon(
                &ui,
                icon,
                Rect::from_center_size(hit.center(), glyph),
                color,
            );
            if response.clicked() && !active {
                if is_calendar {
                    calendar = true;
                } else {
                    action = Some(TabTarget::Box("imbox".into()));
                }
            }
        }
        if calendar {
            self.commands.toast(Toast::new("HEY doesn't include a calendar."));
        }

        // The rest of the rail goes back.
        let back = Rect::from_min_max(
            pos2(rect.left(), switcher.bottom() + 12.0),
            pos2(rect.right() - 2.0, rect.bottom()),
        );
        let response = ui
            .interact(back, egui::Id::new("rail-back"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() {
            action = Some(TabTarget::Box("imbox".into()));
        }

        if let Some(target) = action {
            match target {
                TabTarget::Box(kind) => {
                    if self.thread.is_some() {
                        self.close_thread();
                    }
                    if self.view != View::Mail || self.selected_box != kind {
                        self.select_box(&kind);
                    } else {
                        self.view = View::Mail;
                    }
                }
                TabTarget::Screener => self.open_screener(),
            }
        }
    }

    fn content_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let mut ui = ui.new_child(UiBuilder::new().max_rect(rect));
        ui.set_clip_rect(rect);
        ui.painter().rect_filled(rect, 0.0, self.p.list);
        match self.view {
            View::Mail => {
                if self.thread.is_some() {
                    self.thread_ui(&mut ui, rect);
                } else {
                    self.list_ui(&mut ui, rect);
                }
            }
            View::Screener => self.screener_ui(&mut ui, rect),
            View::Search => self.search_ui(&mut ui, rect),
        }
    }

    // ---------------------------------------------------------------------------------------
    // Thread list.
    // ---------------------------------------------------------------------------------------

    fn list_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let rem = exo_ui::rem(ui.ctx().content_rect().width(), false);
        let view = Rect::from_min_max(
            pos2(rect.left() + VIEW_PAD_X, rect.top()),
            pos2(rect.right() - VIEW_PAD_X, rect.bottom()),
        );
        let compact = ui.ctx().content_rect().width() <= 1152.0;
        let header = Rect::from_min_size(view.min, vec2(view.width(), HEADER));
        let list = Rect::from_min_max(pos2(view.left(), header.bottom()), view.max);

        self.mail_header(ui, header, rem, compact);
        self.list_rows(ui, list, rem, compact);
    }

    fn mail_header(&mut self, ui: &mut Ui, rect: Rect, rem: f32, compact: bool) {
        let p = self.p;
        let painter = ui.painter().clone();

        // Folders.
        let hamburger = pos2(
            rect.left() + if compact { 9.0 } else { 2.5 * rem - 25.0 },
            rect.top() + 36.0,
        );
        let folders_hit = Rect::from_min_size(hamburger - vec2(10.0, 10.0), vec2(36.0, 32.0));
        let folders = ui
            .interact(folders_hit, egui::Id::new("folders"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Boxes and labels");
        let folders_color = if folders.hovered() { p.icon_hover } else { p.icon };
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_hamburgerMenu",
            Rect::from_min_size(hamburger, vec2(16.0, 12.0)),
            folders_color,
        );
        if folders.clicked() {
            self.drawer_open = true;
        }

        // The split tabs: this box, then the Screener (or the Imbox), then Boxes.
        let left = rect.left() + if compact { 44.0 } else { 3.5 * rem };
        let top = rect.top() + 32.0;
        let mut x = left;
        let mut tabs = vec![(
            self.box_name(&self.selected_box),
            self.current_rows().len(),
            TabTarget::Box(self.selected_box.clone()),
            true,
        )];
        if self.selected_box != "imbox" {
            tabs.push(("The Imbox".into(), self.rows.get("imbox").map_or(0, Vec::len), TabTarget::Box("imbox".into()), false));
        } else if !self.screener.is_empty() {
            tabs.push(("The Screener".into(), self.screener.len(), TabTarget::Screener, false));
        } else {
            tabs.push(("The Feed".into(), self.rows.get("feedbox").map_or(0, Vec::len), TabTarget::Box("feedbox".into()), false));
        }

        for (index, (name, count, target, selected)) in tabs.into_iter().enumerate() {
            if index > 0 {
                // The bullet between splits.
                x += 8.0;
                x += TAB.paint_line(&painter, pos2(x, top), "•", p.faint, None) + 8.0;
            }
            let count_text = count.to_string();
            let show_count = count > 0;
            let count_width = if show_count { 5.0 + COUNT.width(ui.ctx(), &count_text) } else { 0.0 };
            let width = TAB.width(ui.ctx(), &name) + count_width;
            let hit = Rect::from_min_size(pos2(x, top), vec2(width, 24.0));
            let response = ui
                .interact(hit, egui::Id::new(("header-tab", index)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            let name_color = if selected {
                p.ink.gamma_multiply(0.8)
            } else if response.hovered() {
                p.ink.gamma_multiply(0.6)
            } else {
                p.ink.gamma_multiply(0.4)
            };
            TAB.paint_line(&painter, pos2(x, top), &name, name_color, None);
            if show_count {
                COUNT.paint_line(
                    &painter,
                    pos2(x + TAB.width(ui.ctx(), &name) + 5.0, top + 1.0),
                    &count_text,
                    if selected { p.faint } else { p.ink.gamma_multiply(0.25) },
                    None,
                );
            }
            if response.clicked() {
                match target {
                    TabTarget::Box(kind) => self.select_box(&kind),
                    TabTarget::Screener => self.open_screener(),
                }
            }
            x += width;
        }

        // Add Split's HEY equivalent.
        x += 1.5 * rem;
        let add_plus = pos2(x, top + 4.75);
        let add_label = "Boxes";
        let add_width = 16.5 + TAB.width(ui.ctx(), add_label);
        let add_hit = Rect::from_min_size(pos2(x, top), vec2(add_width, 24.0));
        let add = ui
            .interact(add_hit, egui::Id::new("add-boxes"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let add_color = if add.hovered() { p.ink.gamma_multiply(0.6) } else { p.ink.gamma_multiply(0.4) };
        let plus = Rect::from_min_size(add_plus, Vec2::splat(10.5));
        let c = plus.center();
        painter.line_segment([pos2(c.x - 4.5, c.y), pos2(c.x + 4.5, c.y)], Stroke::new(1.4, add_color));
        painter.line_segment([pos2(c.x, c.y - 4.5), pos2(c.x, c.y + 4.5)], Stroke::new(1.4, add_color));
        TAB.paint_line(&painter, pos2(x + 16.5, top), add_label, add_color, None);
        if add.clicked() {
            self.drawer_open = true;
        }

        // Compose and search, floated right.
        let search = pos2(rect.right() - 2.0 * rem + 5.0 - 14.0, rect.top() + 35.0);
        let search_hit = Rect::from_min_size(search - vec2(6.0, 10.0), vec2(30.0, 34.0));
        let search_response = ui
            .interact(search_hit, egui::Id::new("header-search"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Search");
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_headerSearch-14",
            Rect::from_min_size(search, Vec2::splat(14.0)),
            if search_response.hovered() { p.icon_hover } else { p.icon },
        );
        if search_response.clicked() {
            self.open_search();
        }

        let compose = pos2(rect.right() - 2.0 * rem - 20.0 - 15.0, rect.top() + 35.0);
        let compose_hit = Rect::from_min_size(compose - vec2(10.0, 10.0), vec2(30.0, 34.0));
        let compose_response = ui
            .interact(compose_hit, egui::Id::new("header-compose"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Compose");
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_headerCompose-14",
            Rect::from_min_size(compose, Vec2::splat(14.0)),
            if compose_response.hovered() { p.icon_hover } else { p.icon },
        );
        if compose_response.clicked() {
            self.apply(Action::Compose);
        }
    }

    fn list_rows(&mut self, ui: &mut Ui, rect: Rect, rem: f32, compact: bool) {
        let p = self.p;
        let rows = self.current_rows().to_vec();
        let labels: HashMap<i64, String> = self.labels.iter().map(|l| (l.id, l.name.clone())).collect();
        let now = jiff::Zoned::now();
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
        child.set_clip_rect(rect);
        let mut first_group = true;
        let mut clicked = None;
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut child, |ui| {
                ui.add_space(LIST_PAD_TOP);
                let width = ui.available_width();
                let mut previous = String::new();
                for (index, row) in rows.iter().enumerate() {
                    let group = day_group(row.created_at.as_deref(), &now);
                    if group != previous {
                        if !first_group {
                            let (item, _) = ui.allocate_exact_size(vec2(width, SEPARATOR), Sense::hover());
                            ROW_BOLD.paint_line(
                                ui.painter(),
                                pos2(item.left(), item.top() + 23.0),
                                &group,
                                p.faint,
                                None,
                            );
                        }
                        first_group = false;
                        previous = group;
                    }
                    if row_ui(
                        ui,
                        row,
                        self.selected == Some(index),
                        self.drawer_open,
                        &labels,
                        rem,
                        compact,
                        p,
                        &now,
                    ) {
                        clicked = Some(index);
                    }
                }
                if rows.is_empty() {
                    ui.add_space(100.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Nothing here yet").size(16.0).color(p.faint));
                        ui.add_space(6.0);
                        ui.label(RichText::new("New mail will show up here as it arrives.").size(12.0).color(p.faint));
                    });
                }
            });

        if let Some(index) = clicked {
            self.selected = Some(index);
            self.open_selected();
        }
    }

    // ---------------------------------------------------------------------------------------
    // Screener and search.
    // ---------------------------------------------------------------------------------------

    fn screener_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.p;
        let rem = exo_ui::rem(ui.ctx().content_rect().width(), false);
        let view = Rect::from_min_max(
            pos2(rect.left() + VIEW_PAD_X, rect.top()),
            pos2(rect.right() - VIEW_PAD_X, rect.bottom()),
        );
        let header = Rect::from_min_size(view.min, vec2(view.width(), HEADER));
        let list = Rect::from_min_max(pos2(view.left(), header.bottom()), view.max);

        let hamburger = pos2(view.left() + 2.5 * rem - 25.0, header.top() + 36.0);
        let folders = ui
            .interact(
                Rect::from_min_size(hamburger - vec2(10.0, 10.0), vec2(36.0, 32.0)),
                egui::Id::new("screener-folders"),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_hamburgerMenu",
            Rect::from_min_size(hamburger, vec2(16.0, 12.0)),
            if folders.hovered() { p.icon_hover } else { p.icon },
        );
        if folders.clicked() {
            self.drawer_open = true;
        }

        let left = view.left() + 3.5 * rem;
        let top = header.top() + 32.0;
        let mut x = left;
        let name = "The Screener";
        let count = self.screener.len().to_string();
        TAB.paint_line(ui.painter(), pos2(x, top), name, p.ink.gamma_multiply(0.8), None);
        x += TAB.width(ui.ctx(), name) + 5.0;
        COUNT.paint_line(ui.painter(), pos2(x, top + 1.0), &count, p.faint, None);
        x += COUNT.width(ui.ctx(), &count) + 8.0;
        TAB.paint_line(ui.painter(), pos2(x, top), "•", p.faint, None);
        x += TAB.width(ui.ctx(), "•") + 8.0;
        let imbox_name = "The Imbox";
        let hit = Rect::from_min_size(pos2(x, top), vec2(TAB.width(ui.ctx(), imbox_name) + 30.0, 24.0));
        let response = ui
            .interact(hit, egui::Id::new("screener-imbox"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        TAB.paint_line(
            ui.painter(),
            pos2(x, top),
            imbox_name,
            if response.hovered() { p.ink.gamma_multiply(0.6) } else { p.ink.gamma_multiply(0.4) },
            None,
        );
        if response.clicked() {
            self.select_box("imbox");
        }

        let items = self.screener.clone();
        let mut child = ui.new_child(UiBuilder::new().max_rect(list));
        child.set_clip_rect(list);
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut child, |ui| {
                ui.add_space(8.0);
                let mut approve = None;
                let mut deny = None;
                let mut open = None;
                for item in &items {
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
                    let hovered = ui.rect_contains_pointer(row);
                    if hovered {
                        ui.painter().rect_filled(row.shrink2(vec2(0.0, 2.0)), 2.0, p.hover);
                    }
                    let left = row.left() + 2.0 * rem;
                    let text_top = row.top() + 14.0;
                    let name = if item.sender_name.is_empty() { &item.sender_email } else { &item.sender_name };
                    ROW_BOLD.paint_line(ui.painter(), pos2(left, text_top), name, p.ink, None);
                    let subject_x = left + 0.225 * (row.width() - 2.0 * rem) + 8.0;
                    ROW_TEXT.paint_line(ui.painter(), pos2(subject_x, text_top), &item.subject, p.ink, None);
                    ROW_TEXT.paint_line(
                        ui.painter(),
                        pos2(left, text_top + 18.0),
                        &item.summary,
                        p.muted,
                        Some(row.width() - 4.0 * rem),
                    );

                    let let_in = Rect::from_min_size(pos2(row.right() - 2.0 * rem - 76.0, row.top() + 18.0), vec2(76.0, 28.0));
                    let turn_away = Rect::from_min_size(pos2(let_in.left() - 104.0, row.top() + 18.0), vec2(96.0, 28.0));
                    let let_in_response = ui
                        .interact(let_in, egui::Id::new(("screen-in", item.clearance_id)), Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand);
                    ui.painter().rect_filled(
                        let_in,
                        egui::CornerRadius::same(4),
                        if let_in_response.hovered() { p.accent.gamma_multiply(0.9) } else { p.accent },
                    );
                    ui.painter().text(
                        let_in.center(),
                        Align2::CENTER_CENTER,
                        "Let in",
                        FontId::new(12.0, FontFamily::Proportional),
                        p.accent_ink,
                    );
                    let deny_response = ui
                        .interact(turn_away, egui::Id::new(("screen-out", item.clearance_id)), Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand);
                    ui.painter().text(
                        turn_away.center(),
                        Align2::CENTER_CENTER,
                        "Turn away",
                        FontId::new(12.0, FontFamily::Proportional),
                        if deny_response.hovered() { p.ink } else { p.muted },
                    );
                    if let_in_response.clicked() {
                        approve = Some(item.clearance_id);
                    }
                    if deny_response.clicked() {
                        deny = Some(item.clearance_id);
                    }
                    if let Some(topic) = (ui.rect_contains_pointer(row) && ui.input(|i| i.pointer.primary_clicked()) && item.topic_id != 0).then_some(item.topic_id) {
                        if !let_in.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or(Pos2::ZERO))
                            && !turn_away.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or(Pos2::ZERO))
                        {
                            open = Some(topic);
                        }
                    }
                }
                if let Some(id) = approve {
                    self.send(BackendCommand::Screen(id, true));
                }
                if let Some(id) = deny {
                    self.send(BackendCommand::Screen(id, false));
                }
                if let Some(topic) = open {
                    self.open_topic(topic);
                }
                if items.is_empty() {
                    ui.add_space(90.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Nobody is waiting").size(17.0).color(p.faint));
                        ui.add_space(6.0);
                        ui.label(RichText::new("New senders appear here before reaching your Imbox.").size(12.0).color(p.faint));
                    });
                }
            });
    }

    fn search_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.p;
        let view = Rect::from_min_max(
            pos2(rect.left() + VIEW_PAD_X, rect.top()),
            pos2(rect.right() - VIEW_PAD_X, rect.bottom()),
        );
        let header = Rect::from_min_size(view.min, vec2(view.width(), HEADER));
        let list = Rect::from_min_max(pos2(view.left(), header.bottom()), view.max);

        let icon = Rect::from_min_size(pos2(header.left() + 3.5 * 13.5, header.top() + 35.0), Vec2::splat(14.0));
        exo_ui::paint_icon(ui, "legacy/DEPRECATED_headerSearch-14", icon, p.icon);
        let field = Rect::from_min_size(pos2(icon.right() + 12.0, header.top() + 30.0), vec2((header.width() - 160.0).max(200.0), 28.0));
        let response = ui.put(
            field,
            egui::TextEdit::singleline(&mut self.search_text)
                .frame(egui::Frame::NONE)
                .hint_text(RichText::new("Search all mail").size(16.0).color(p.faint))
                .font(FontId::new(16.0, FontFamily::Proportional))
                .text_color(p.ink)
                .desired_width(field.width()),
        );
        if self.search_focus {
            response.request_focus();
            self.search_focus = false;
        }
        let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter {
            let query = self.search_text.clone();
            if !query.trim().is_empty() {
                self.send(BackendCommand::Search(query));
            }
        }
        let close = Rect::from_min_size(pos2(header.right() - 44.0, header.top() + 32.0), Vec2::splat(24.0));
        let close_response = ui
            .interact(close, egui::Id::new("search-close"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Close search");
        exo_ui::paint_icon(ui, "shell/tab-close", Rect::from_center_size(close.center(), Vec2::splat(10.0)), p.muted);
        if close_response.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.view = View::Mail;
        }

        let hits = self.hits.clone().unwrap_or_default();
        let now = jiff::Zoned::now();
        let mut child = ui.new_child(UiBuilder::new().max_rect(list));
        child.set_clip_rect(list);
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut child, |ui| {
                ui.add_space(2.0);
                for (index, hit) in hits.iter().enumerate() {
                    let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(row, 2.0, p.hover);
                    }
                    let left = row.left() + 44.0;
                    ROW_BOLD.paint_line(ui.painter(), pos2(left, row.top() + 9.0), &hit.sender_name, p.ink, Some(180.0));
                    let subject_x = left + 0.225 * (row.width() - 60.0);
                    ROW_TEXT.paint_line(ui.painter(), pos2(subject_x, row.top() + 9.0), &hit.subject, p.ink, Some(row.width() - (subject_x - row.left()) - 100.0));
                    ROW_TEXT.paint_line(ui.painter(), pos2(left, row.top() + 26.0), &hit.summary, p.muted, Some(row.width() - 80.0));
                    let date = row_date(hit.updated_at.as_deref(), &now);
                    let date_width = ROW_DATE.width(ui.ctx(), &date);
                    ROW_DATE.paint_line(ui.painter(), pos2(row.right() - 21.0 - date_width, row.top() + 9.0), &date, p.muted, None);
                    if response.clicked() {
                        self.open_topic(hit.topic_id);
                    }
                    let _ = index;
                }
                if self.hits.is_none() {
                    ui.add_space(80.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Search all of your mail").size(16.0).color(p.faint));
                    });
                } else if hits.is_empty() {
                    ui.add_space(80.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("No matches").size(16.0).color(p.faint));
                    });
                }
            });
    }

    // ---------------------------------------------------------------------------------------
    // An open conversation.
    // ---------------------------------------------------------------------------------------

    fn thread_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.p;
        let Some(thread) = self.thread.clone() else { return };
        let rem = exo_ui::rem(ui.ctx().content_rect().width(), false);
        let top = rect.top() + 24.0;
        let subject_left = rect.left() + 4.0 * rem;

        // Back gutter.
        let back = Rect::from_min_size(pos2(rect.left() + 20.0, rect.top() + 22.0), vec2(28.0, 28.0));
        let back_response = ui
            .interact(back, egui::Id::new("thread-back"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Back");
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_leftChevronLarge",
            Rect::from_center_size(back.center(), Vec2::splat(14.0)),
            if back_response.hovered() { p.icon_hover } else { p.icon },
        );
        if back_response.clicked() {
            self.close_thread();
            return;
        }

        let subject_width = (rect.width() - 8.0 * rem - 200.0).max(200.0);
        SUBJECT.paint_line(ui.painter(), pos2(subject_left, top), &thread.subject, p.ink, Some(subject_width));

        // Actions, kept quiet and floated right.
        let mut right = rect.right() - 4.0 * rem;
        let action = |label: &str, right: &mut f32| {
            let width = DATA.width(ui.ctx(), label);
            let hit = Rect::from_min_size(pos2(*right - width, top + 4.0), vec2(width, 20.0));
            let response = ui
                .interact(hit, egui::Id::new(("thread-action", label)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            DATA.paint_line(
                ui.painter(),
                pos2(*right - width, top + 4.0),
                label,
                if response.hovered() { p.ink } else { p.muted },
                None,
            );
            *right -= width + 16.0;
            response.clicked()
        };
        if action("Trash", &mut right) {
            self.apply(Action::Trash);
            return;
        }
        if action("Set Aside", &mut right) {
            self.apply(Action::SetAside);
            return;
        }
        if action("Reply", &mut right) {
            self.reply_open = true;
        }

        let header_bottom = rect.top() + 76.0;
        ui.painter().hline(
            rect.x_range(),
            header_bottom,
            Stroke::new(1.0, p.line),
        );
        let area = Rect::from_min_max(pos2(rect.left(), header_bottom), rect.max);
        let card_width = (rect.width() - 8.0 * rem).min(760.0).max(320.0);
        let card_left = rect.left() + 4.0 * rem;
        let body_width = (card_width - 64.0).max(200.0);

        // Layout every body before scrolling, so the scroll closure doesn't borrow the app.
        let bodies: Vec<Option<(egui::TextureHandle, Vec2)>> = thread
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                self.body.paint(
                    ui.ctx(),
                    thread.topic_id as u64 * 1000 + index as u64,
                    &message.html,
                    body_width,
                    p.dark,
                )
            })
            .collect();

        let mut child = ui.new_child(UiBuilder::new().max_rect(area));
        child.set_clip_rect(area);
        let mut send_reply = false;
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut child, |ui| {
                ui.add_space(20.0);
                for (index, message) in thread.messages.iter().enumerate() {
                    let card = egui::Frame::new()
                        .fill(p.card)
                        .corner_radius(egui::CornerRadius::same(4))
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 1],
                            blur: 4,
                            spread: 0,
                            color: Color32::from_black_alpha(if p.dark { 90 } else { 38 }),
                        })
                        .outer_margin(egui::Margin {
                            left: (card_left - area.left()) as i8,
                            right: 0,
                            top: 0,
                            bottom: 0,
                        })
                        .inner_margin(egui::Margin {
                            left: 32,
                            right: 32,
                            top: 16,
                            bottom: 20,
                        });
                    card.show(ui, |ui| {
                        ui.set_width(body_width);
                        let content = ui.max_rect();
                        let name = if message.sender_name.is_empty() {
                            &message.sender_email
                        } else {
                            &message.sender_name
                        };
                        THREAD_FROM.paint_line(
                            ui.painter(),
                            content.min,
                            name,
                            p.ink,
                            Some((content.width() - 120.0).max(80.0)),
                        );
                        let date = short_thread_date(message.sent_at.as_deref());
                        let date_width = THREAD_DATE.width(ui.ctx(), &date);
                        THREAD_DATE.paint_line(
                            ui.painter(),
                            pos2(content.right() - date_width, content.top()),
                            &date,
                            p.muted,
                            None,
                        );
                        ui.add_space(36.0);
                        if let Some((texture, size)) = &bodies[index] {
                            let (image_rect, _) = ui.allocate_exact_size(*size, Sense::hover());
                            ui.painter().image(
                                texture.id(),
                                image_rect,
                                Rect::from_min_size(Pos2::ZERO, vec2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.label(RichText::new("(empty message)").italics().color(p.faint));
                        }
                    });
                    ui.add_space(20.0);
                }

                let reply = egui::Frame::new()
                    .fill(p.card)
                    .corner_radius(egui::CornerRadius::same(4))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 1],
                        blur: 4,
                        spread: 0,
                        color: Color32::from_black_alpha(if p.dark { 90 } else { 38 }),
                    })
                    .outer_margin(egui::Margin {
                        left: (card_left - area.left()) as i8,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    })
                    .inner_margin(egui::Margin::same(16));
                reply.show(ui, |ui| {
                    ui.set_width(body_width);
                    ui.label(RichText::new("Reply").size(12.0).color(p.muted));
                    ui.add_space(6.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.reply_text)
                            .desired_rows(if self.reply_open { 5 } else { 2 })
                            .hint_text(RichText::new("Write a reply…").color(p.faint))
                            .desired_width(body_width)
                            .frame(egui::Frame::NONE),
                    );
                    ui.add_space(8.0);
                    let send = egui::Button::new(
                        RichText::new("Send reply").size(12.0).color(p.accent_ink),
                    )
                    .fill(p.accent)
                    .min_size(vec2(92.0, 30.0));
                    if ui.add(send).clicked() {
                        send_reply = true;
                    }
                });
                ui.add_space(32.0);
            });

        if send_reply && let Some(message) = thread.messages.last() {
            let html = exo_mail::text_to_html(&self.reply_text);
            self.send(BackendCommand::Reply {
                entry_id: message.id,
                body: html,
            });
            self.reply_text.clear();
            self.reply_open = false;
        }
    }

    // ---------------------------------------------------------------------------------------
    // The right-hand contact pane.
    // ---------------------------------------------------------------------------------------

    fn contact_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.p;
        let rem = exo_ui::rem(ui.ctx().content_rect().width(), false);
        let mut ui = ui.new_child(UiBuilder::new().max_rect(rect));
        ui.set_clip_rect(rect);
        ui.painter().rect_filled(rect, 0.0, p.right);
        ui.painter().vline(rect.left() + 0.5, rect.y_range(), Stroke::new(1.0, p.line));

        let left = rect.left() + 1.0 + 2.0 * rem;
        let width = rect.width() - 4.0 * rem;
        let name_top = rect.top() + 32.0;

        // Who the pane is about: the open thread's first sender, else the cursor's row, else the
        // first screener item, else the account itself.
        let (name, email, current_topic) = if let Some(thread) = &self.thread {
            let first = thread.messages.first();
            (
                first
                    .map(|m| if m.sender_name.is_empty() { m.sender_email.clone() } else { m.sender_name.clone() })
                    .unwrap_or_else(|| thread.subject.clone()),
                first.map(|m| m.sender_email.clone()).unwrap_or_default(),
                Some(thread.topic_id),
            )
        } else if let Some(row) = self.current_posting() {
            (row.display_name().to_string(), row.sender_email.clone(), Some(row.topic_id))
        } else if self.view == View::Screener {
            self.screener
                .first()
                .map(|s| {
                    (
                        if s.sender_name.is_empty() { s.sender_email.clone() } else { s.sender_name.clone() },
                        s.sender_email.clone(),
                        Some(s.topic_id),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        if self.identity.name.is_empty() { "Exo".into() } else { self.identity.name.clone() },
                        self.identity.email.clone(),
                        None,
                    )
                })
        } else {
            (
                if self.identity.name.is_empty() { "Exo".into() } else { self.identity.name.clone() },
                self.identity.email.clone(),
                None,
            )
        };

        let name_width = NAME.width(ui.ctx(), &name);
        let name_hit = Rect::from_min_size(pos2(left, name_top), vec2(name_width, 24.0));
        let name_response = ui
            .interact(name_hit, egui::Id::new("contact-name"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Search for this person");
        NAME.paint_line(
            ui.painter(),
            pos2(left, name_top),
            &name,
            if name_response.hovered() { p.ink } else { p.ink.gamma_multiply(0.8) },
            Some(width),
        );
        if name_response.clicked() && !email.is_empty() {
            self.search_text = email.clone();
            self.view = View::Search;
            self.thread = None;
            self.send(BackendCommand::Search(email.clone()));
        }

        let email_top = rect.top() + 89.0;
        let email_width = BOLD.width(ui.ctx(), &email);
        let email_hit = Rect::from_min_size(pos2(left, email_top), vec2(email_width, 16.0));
        let email_response = ui
            .interact(email_hit, egui::Id::new("contact-email"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Compose to this address");
        BOLD.paint_line(
            ui.painter(),
            pos2(left, email_top),
            &email,
            p.ink.gamma_multiply(0.55),
            Some(width),
        );
        if email_response.hovered() {
            ui.painter().hline(
                email_hit.x_range(),
                email_top + BOLD.baseline(ui.ctx()) + 1.5,
                Stroke::new(1.0, p.ink.gamma_multiply(0.55)),
            );
        }
        if email_response.clicked() && !email.is_empty() {
            self.compose = Some(ComposeDraft { to: email.clone(), ..Default::default() });
            self.compose_focus = true;
        }

        // Recent conversations with this person, from every box.
        let mut recent: Vec<(i64, String)> = Vec::new();
        let mut seen = HashSet::new();
        if !email.is_empty() {
            for rows in self.rows.values() {
                for row in rows {
                    if row.sender_email.eq_ignore_ascii_case(&email)
                        && Some(row.topic_id) != current_topic
                        && seen.insert(row.topic_id)
                    {
                        recent.push((row.topic_id, row.subject.clone()));
                    }
                }
            }
        }
        recent.truncate(4);

        let mut section_top = rect.top() + 124.0;
        let mut open_topic = None;
        let mut search_email = false;
        if !recent.is_empty() && email != self.identity.email {
            let header = Rect::from_min_size(pos2(left, section_top), vec2(width, 16.0));
            let header_response = ui
                .interact(header, egui::Id::new("contact-mail"), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Search this person's mail");
            section_icon(&ui, left, section_top, "legacy/DEPRECATED_mail", p, header_response.hovered());
            BOLD.paint_line(
                ui.painter(),
                pos2(left + 24.0, section_top + 1.0),
                "Mail",
                p.ink.gamma_multiply(0.8),
                None,
            );
            if header_response.clicked() {
                search_email = true;
            }
            for (index, (topic, subject)) in recent.iter().enumerate() {
                let row = Rect::from_min_size(pos2(left, section_top + 25.0 + 24.0 * index as f32), vec2(width, 16.0));
                let response = ui
                    .interact(row, egui::Id::new(("contact-thread", index)), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand);
                DATA.paint_line(
                    ui.painter(),
                    row.min,
                    subject,
                    if response.hovered() { p.ink.gamma_multiply(0.8) } else { p.ink.gamma_multiply(0.55) },
                    Some(width),
                );
                if response.clicked() {
                    open_topic = Some(*topic);
                }
            }
            section_top += 16.0 + 8.0 + 24.0 * recent.len() as f32 + 20.0;
        }

        let domain = email
            .rsplit_once('@')
            .map(|(_, domain)| domain.to_lowercase())
            .filter(|domain| !is_popular_domain(domain));
        let mut open_domain = None;
        if let Some(domain) = domain {
            let hit = Rect::from_min_size(pos2(left, section_top), vec2(24.0 + BOLD.width(ui.ctx(), &domain), 16.0));
            let response = ui
                .interact(hit, egui::Id::new("contact-domain"), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            section_icon(&ui, left, section_top, "legacy/DEPRECATED_homepage", p, response.hovered());
            BOLD.paint_line(
                ui.painter(),
                pos2(left + 24.0, section_top + 1.0),
                &domain,
                p.ink.gamma_multiply(if response.hovered() { 0.9 } else { 0.8 }),
                None,
            );
            if response.clicked() {
                open_domain = Some(domain);
            }
        }

        // The footer: sync status, calendar and settings.
        let footer_top = rect.bottom() - 12.0 - 24.0;
        let sync = if self.status.is_empty() { String::new() } else { self.status.clone() };
        if !sync.is_empty() {
            DATA.paint_line(
                ui.painter(),
                pos2(left, rect.bottom() - 27.0 - DATA.line_height),
                &sync,
                p.faint,
                Some(width - 80.0),
            );
        }
        let footer_right = rect.right() - 2.0 * rem + 5.0;
        let calendar_hit = Rect::from_min_max(
            pos2(footer_right - 51.0, footer_top + 1.0),
            pos2(footer_right - 25.0, footer_top + 23.0),
        );
        let calendar_response = ui
            .interact(calendar_hit, egui::Id::new("footer-calendar"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Calendar");
        exo_ui::paint_icon(
            &ui,
            "legacy/DEPRECATED_calendar",
            Rect::from_center_size(calendar_hit.center(), Vec2::splat(12.0)),
            if calendar_response.hovered() { p.icon_hover } else { p.icon },
        );
        if calendar_response.clicked() {
            self.commands.toast(Toast::new("HEY doesn't include a calendar."));
        }

        let gear_hit = Rect::from_min_max(
            pos2(footer_right - 25.0, footer_top),
            pos2(footer_right, footer_top + 24.0),
        );
        let gear_response = ui
            .interact(gear_hit, egui::Id::new("footer-settings"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Settings");
        exo_ui::paint_icon(
            &ui,
            "legacy/DEPRECATED_settings",
            Rect::from_center_size(gear_hit.center(), Vec2::splat(14.0)),
            if gear_response.hovered() || self.settings_open { p.icon_hover } else { p.icon },
        );
        if gear_response.clicked() {
            self.settings_open = !self.settings_open;
        }

        if search_email && !email.is_empty() {
            self.search_text = email.clone();
            self.view = View::Search;
            self.thread = None;
            self.send(BackendCommand::Search(email));
        }
        if let Some(topic) = open_topic {
            self.open_topic(topic);
        }
        if let Some(domain) = open_domain {
            ui.ctx().open_url(egui::OpenUrl::new_tab(format!("https://{domain}")));
        }
    }

    // ---------------------------------------------------------------------------------------
    // The box/label drawer and the settings menu.
    // ---------------------------------------------------------------------------------------

    fn drawer_ui(&mut self, ui: &mut Ui, content: Rect) {
        let p = self.p;
        let mut ui = ui.new_child(UiBuilder::new().max_rect(content));
        ui.set_clip_rect(content);
        let scrim = ui.interact(content, egui::Id::new("drawer-scrim"), Sense::click());
        if scrim.clicked() {
            self.drawer_open = false;
            return;
        }

        let width = (0.3 * content.width()).clamp(300.0, 416.0).min(content.width());
        let drawer = Rect::from_min_size(content.min, vec2(width, content.height()));
        exo_ui::BoxShadow::new([0.0, 0.0], 12.0, 0.0, p.shadow)
            .paint(ui.painter(), drawer, egui::CornerRadius::ZERO);
        ui.painter().rect_filled(drawer, 0.0, p.panel);
        ui.painter().vline(drawer.right() - 0.5, drawer.y_range(), Stroke::new(1.0, p.line));

        let left = drawer.left() + 20.0;
        NAME.paint_line(ui.painter(), pos2(left, drawer.top() + 24.0), "Boxes", p.ink, None);
        let close = Rect::from_min_size(pos2(drawer.right() - 40.0, drawer.top() + 26.0), Vec2::splat(20.0));
        let close_response = ui
            .interact(close, egui::Id::new("drawer-close"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        exo_ui::paint_icon(
            &ui,
            "shell/tab-close",
            Rect::from_center_size(close.center(), Vec2::splat(10.0)),
            if close_response.hovered() { p.icon_hover } else { p.icon },
        );
        if close_response.clicked() {
            self.drawer_open = false;
            return;
        }

        let mut y = drawer.top() + 64.0;
        let mut chosen = None;
        for kind in BOX_ORDER {
            let active = self.view == View::Mail && self.selected_box == *kind;
            let row = Rect::from_min_size(pos2(drawer.left() + 8.0, y), vec2(drawer.width() - 16.0, 34.0));
            let response = ui
                .interact(row, egui::Id::new(("drawer-box", *kind)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            if active {
                ui.painter().rect_filled(row, egui::CornerRadius::same(2), p.selected);
                ui.painter().rect_filled(
                    Rect::from_min_size(row.left_top(), vec2(3.0, row.height())),
                    egui::CornerRadius::same(1),
                    p.accent,
                );
            } else if response.hovered() {
                ui.painter().rect_filled(row, egui::CornerRadius::same(2), p.hover);
            }
            DATA.paint_line(
                ui.painter(),
                pos2(row.left() + 12.0, row.top() + 9.0),
                &self.box_name(kind),
                if active { p.ink } else { p.ink.gamma_multiply(0.65) },
                None,
            );
            let count = self.rows.get(*kind).map_or(0, Vec::len);
            if count > 0 {
                let text = count.to_string();
                let w = DATA.width(ui.ctx(), &text);
                DATA.paint_line(
                    ui.painter(),
                    pos2(row.right() - 12.0 - w, row.top() + 9.0),
                    &text,
                    p.faint,
                    None,
                );
            }
            if response.clicked() {
                chosen = Some(kind.to_string());
            }
            y += 34.0;
        }

        y += 18.0;
        SECTION.paint_line(ui.painter(), pos2(left, y), "LABELS", p.faint, None);
        y += 20.0;
        let mut chosen_label = None;
        for label in &self.labels {
            let row = Rect::from_min_size(pos2(drawer.left() + 8.0, y), vec2(drawer.width() - 16.0, 30.0));
            let response = ui
                .interact(row, egui::Id::new(("drawer-label", label.id)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            if response.hovered() {
                ui.painter().rect_filled(row, egui::CornerRadius::same(2), p.hover);
            }
            let (fill, _) = exo_ui::label_colors(&label.name, self.p.exo_theme());
            ui.painter().circle_filled(pos2(row.left() + 14.0, row.center().y), 4.0, fill);
            DATA.paint_line(
                ui.painter(),
                pos2(row.left() + 28.0, row.top() + 7.0),
                &label.name,
                p.ink.gamma_multiply(0.75),
                None,
            );
            if response.clicked() {
                chosen_label = Some(label.id);
            }
            y += 30.0;
        }
        if self.labels.is_empty() {
            DATA.paint_line(
                ui.painter(),
                pos2(left, y),
                "No labels yet",
                p.faint,
                Some(drawer.width() - 40.0),
            );
        }

        if let Some(kind) = chosen {
            self.select_box(&kind);
        } else if let Some(id) = chosen_label {
            if self.current_posting().is_some() {
                self.apply(Action::Label(id));
                self.drawer_open = false;
            } else {
                self.commands
                    .toast(Toast::new("Select a conversation first, then pick a label."));
            }
        }
    }

    fn settings_ui(&mut self, ui: &mut Ui, pane: Rect) {
        let p = self.p;
        let width = 180.0;
        let height = 32.0 * 3.0 + 12.0;
        let popup = Rect::from_min_size(
            pos2(pane.right() - 40.0 - width, pane.bottom() - 64.0 - height),
            vec2(width, height),
        );
        let mut ui = ui.new_child(UiBuilder::new().max_rect(popup));
        ui.set_clip_rect(popup);
        exo_ui::BoxShadow::new([0.0, 6.0], 18.0, 0.0, p.shadow)
            .paint(ui.painter(), popup, egui::CornerRadius::same(6));
        ui.painter().rect_filled(popup, egui::CornerRadius::same(6), p.panel);
        ui.painter().rect_stroke(
            popup,
            egui::CornerRadius::same(6),
            Stroke::new(1.0, p.line),
            egui::StrokeKind::Inside,
        );

        let items = ["Refresh", "Toggle light/dark", "Sign out"];
        let mut chosen = None;
        for (index, label) in items.iter().enumerate() {
            let row = Rect::from_min_size(pos2(popup.left(), popup.top() + 6.0 + 32.0 * index as f32), vec2(width, 32.0));
            let response = ui
                .interact(row, egui::Id::new(("settings-row", index)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            if response.hovered() {
                ui.painter().rect_filled(row.shrink2(vec2(6.0, 0.0)), 4.0, p.hover);
            }
            DATA.paint_line(
                ui.painter(),
                pos2(row.left() + 14.0, row.top() + 8.0),
                label,
                if *label == "Sign out" { p.danger } else { p.ink },
                None,
            );
            if response.clicked() {
                chosen = Some(index);
            }
        }
        if let Some(index) = chosen {
            self.settings_open = false;
            match index {
                0 => self.apply(Action::Refresh),
                1 => self.apply(Action::ToggleTheme),
                _ => self.apply(Action::SignOut),
            }
        } else if ui.ctx().input(|i| i.pointer.any_click())
            && ui
                .ctx()
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|at| !popup.contains(at))
        {
            self.settings_open = false;
        }
    }

    // ---------------------------------------------------------------------------------------
    // Sign-in and compose.
    // ---------------------------------------------------------------------------------------

    fn sign_in_ui(&mut self, ui: &mut Ui) {
        let p = self.p;
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 130.0).max(40.0));
            ui.label(RichText::new("EXO").size(13.0).color(p.faint));
            ui.add_space(10.0);
            ui.label(RichText::new("Your mail, at the speed of thought.").size(34.0).color(p.ink));
            ui.add_space(10.0);
            ui.label(RichText::new("A native client for HEY — local-first, keyboard-driven, and quietly fast.").size(14.0).color(p.muted));
            ui.add_space(32.0);
            let button = egui::Button::new(
                RichText::new(if self.starting { "Waiting for HEY…" } else { "Sign in with HEY" })
                    .size(15.0)
                    .color(p.accent_ink),
            )
            .fill(p.accent)
            .min_size(Vec2::new(240.0, 42.0));
            if ui.add(button).clicked() {
                self.starting = true;
                self.send(BackendCommand::SignIn);
            }
            ui.add_space(16.0);
            ui.label(RichText::new("Your browser opens to approve access. Tokens stay on this device.").size(12.0).color(p.faint));
        });
    }

    fn compose_ui(&mut self, ui: &mut Ui, mut compose: ComposeDraft) {
        let p = self.p;
        let mut send = false;
        let mut draft = false;
        let mut close = false;
        egui::Frame::new()
            .fill(p.panel)
            .inner_margin(egui::Margin {
                left: 24,
                right: 24,
                top: 18,
                bottom: 18,
            })
            .show(ui, |ui| {
                ui.set_width(612.0);
                ui.horizontal(|ui| {
                    NAME.paint_line(ui.painter(), ui.cursor().min, "New message", p.ink, None);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let hit = Rect::from_min_size(
                            pos2(ui.max_rect().right() - 24.0, ui.cursor().min.y),
                            Vec2::splat(24.0),
                        );
                        let response = ui
                            .interact(hit, egui::Id::new("compose-close"), Sense::click())
                            .on_hover_cursor(CursorIcon::PointingHand);
                        exo_ui::paint_icon(
                            ui,
                            "shell/tab-close",
                            Rect::from_center_size(hit.center(), Vec2::splat(10.0)),
                            if response.hovered() { p.icon_hover } else { p.icon },
                        );
                        if response.clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(14.0);

                let label_width = 58.0;
                ui.horizontal(|ui| {
                    DATA.paint_line(
                        ui.painter(),
                        pos2(ui.cursor().min.x, ui.cursor().min.y + 7.0),
                        "To",
                        p.faint,
                        None,
                    );
                    ui.add_space(label_width);
                    ui.add(
                        egui::TextEdit::singleline(&mut compose.to)
                            .frame(egui::Frame::NONE)
                            .hint_text(RichText::new("Recipients").color(p.faint))
                            .font(FontId::new(14.0, FontFamily::Proportional))
                            .text_color(p.ink)
                            .desired_width(ui.available_width()),
                    );
                });
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().min.y + 2.0,
                    Stroke::new(1.0, p.line),
                );
                ui.add_space(9.0);
                ui.horizontal(|ui| {
                    DATA.paint_line(
                        ui.painter(),
                        pos2(ui.cursor().min.x, ui.cursor().min.y + 7.0),
                        "Subject",
                        p.faint,
                        None,
                    );
                    ui.add_space(label_width);
                    ui.add(
                        egui::TextEdit::singleline(&mut compose.subject)
                            .frame(egui::Frame::NONE)
                            .hint_text(RichText::new("Subject").color(p.faint))
                            .font(FontId::new(14.0, FontFamily::Proportional))
                            .text_color(p.ink)
                            .desired_width(ui.available_width()),
                    );
                });
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().min.y + 2.0,
                    Stroke::new(1.0, p.line),
                );
                ui.add_space(14.0);
                ui.add(
                    egui::TextEdit::multiline(&mut compose.body)
                        .desired_rows(14)
                        .hint_text(RichText::new("Write your message…").color(p.faint))
                        .font(FontId::new(14.0, FontFamily::Proportional))
                        .text_color(p.ink)
                        .desired_width(f32::INFINITY)
                        .frame(egui::Frame::NONE),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(
                        egui::Button::new(RichText::new("Send").size(12.5).color(p.accent_ink))
                            .fill(p.accent)
                            .min_size(Vec2::new(90.0, 32.0)),
                    ).clicked() {
                        send = true;
                    }
                    if ui.button(RichText::new("Save draft").size(12.0).color(p.ink)).clicked() {
                        draft = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("Discard").size(11.5).color(p.faint)).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if close {
            self.compose = None;
            return;
        }
        self.compose = Some(compose);
        if send || draft {
            let current = self.compose.clone().unwrap();
            let to = split_addresses(&current.to);
            if to.is_empty() {
                self.error = Some("Add at least one recipient".into());
                return;
            }
            let command = if send {
                BackendCommand::Send {
                    to,
                    subject: current.subject,
                    body: exo_mail::text_to_html(&current.body),
                }
            } else {
                BackendCommand::SaveDraft {
                    to,
                    subject: current.subject,
                    body: exo_mail::text_to_html(&current.body),
                }
            };
            self.send(command);
            self.compose = None;
        }
    }
}

// -------------------------------------------------------------------------------------------
// Row painting.
// -------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn row_ui(
    ui: &mut Ui,
    row: &ThreadRow,
    selected: bool,
    blocked: bool,
    labels: &HashMap<i64, String>,
    rem: f32,
    compact: bool,
    p: Palette,
    now: &jiff::Zoned,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), ROW),
        Sense::click(),
    );
    if selected {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(2), p.selected);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), vec2(3.0, rect.height())),
            egui::CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 },
            p.accent,
        );
    } else if response.hovered() && !blocked {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(2), p.hover);
    }

    let padding_left = if compact { 44.0 } else { 3.5 * rem };
    let padding_right = 2.0 * rem - 6.0;
    let left = rect.left() + padding_left;
    let right = rect.right() - padding_right;
    let from_width = 0.225 * (rect.width() - padding_left - padding_right);
    let text_top = rect.top() + 10.0;
    let painter = ui.painter();

    // Status: the unread dot, exactly as the design system paints it.
    let origin = pos2(left, rect.top() + 10.5);
    let unread = if row.seen { exo_ui::Unread::No } else { exo_ui::Unread::Yes };
    exo_ui::paint_status(painter, origin, &exo_ui::Status::resting(unread, false));

    // The sender.
    let name = row.display_name();
    let name_galley = ROW_BOLD.single_line(painter, name, p.ink, Some((from_width - 18.0).max(20.0)));
    exo_ui::paint_text(painter, pos2(left, text_top + 1.0), name_galley, &ROW_BOLD);

    // Label chips, then the subject, then the summary.
    let mut x = left + from_width;
    for label_id in &row.label_ids {
        let Some(label) = labels.get(label_id) else { continue };
        let chip = exo_ui::LabelChip::new(painter, label, false);
        let width = chip.width();
        chip.paint(painter, pos2(x, text_top), 0.0);
        x += width + 6.0;
    }

    let date = row_date(row.created_at.as_deref(), now);
    let date_width = ROW_DATE.width(ui.ctx(), &date);
    let date_x = right - date_width;
    let meta_left = date_x - 8.0;

    let subject_max = (meta_left - x).max(0.0);
    let subject = ROW_TEXT.single_line(painter, &row.subject, p.ink, Some(subject_max));
    let subject_width = subject.size().x;
    exo_ui::paint_text(painter, pos2(x, text_top + 1.0), subject, &ROW_TEXT);

    let summary_left = x + subject_width + rem;
    let summary_max = meta_left - summary_left;
    if summary_max > 12.0 && !row.summary.is_empty() {
        let summary = ROW_TEXT.single_line(painter, &row.summary, p.muted, Some(summary_max));
        exo_ui::paint_text(painter, pos2(summary_left, text_top + 1.0), summary, &ROW_TEXT);
    }

    ROW_DATE.paint_line(
        painter,
        pos2(date_x, text_top),
        &date,
        p.muted,
        None,
    );

    response.clicked() && !blocked
}

fn section_icon(ui: &Ui, left: f32, top: f32, icon: &str, p: Palette, hovered: bool) {
    let disc = exo_ui::pixel_snap(Rect::from_min_size(pos2(left, top), Vec2::splat(16.0)));
    let fill = if hovered {
        Color32::from_rgb(0x92, 0x9d, 0xac)
    } else if p.dark {
        Color32::from_rgb(0x55, 0x5b, 0x64)
    } else {
        p.disc
    };
    let glyph = if p.dark {
        Color32::from_rgb(0x18, 0x19, 0x1a)
    } else {
        Color32::WHITE
    };
    exo_ui::paint_icon_two_tone(ui, icon, disc, glyph, fill);
}

// -------------------------------------------------------------------------------------------
// Dates.
// -------------------------------------------------------------------------------------------

fn zoned(value: Option<&str>) -> Option<jiff::Zoned> {
    let value = value?;
    let timestamp: jiff::Timestamp = value.parse().ok()?;
    Some(timestamp.to_zoned(jiff::tz::TimeZone::system()))
}

fn clock(zoned: &jiff::Zoned) -> String {
    let hour = match zoned.hour() % 12 {
        0 => 12,
        hour => hour,
    };
    let meridiem = if zoned.hour() < 12 { "AM" } else { "PM" };
    format!("{hour}:{:02} {meridiem}", zoned.minute())
}

/// A row's timestamp: the time today, `NOV 15` this year, `11/15/2025` before.
fn row_date(value: Option<&str>, now: &jiff::Zoned) -> String {
    let Some(at) = zoned(value) else { return String::new() };
    if at.date() == now.date() {
        return clock(&at);
    }
    if at.year() == now.year() {
        let month = month_name(at.month())[..3].to_uppercase();
        return format!("{month} {}", at.day());
    }
    format!("{}/{}/{}", at.month(), at.day(), at.year())
}

fn short_thread_date(value: Option<&str>) -> String {
    let Some(at) = zoned(value) else { return String::new() };
    let now = jiff::Zoned::now();
    if at.date() == now.date() {
        clock(&at)
    } else if at.year() == now.year() {
        let month = month_name(at.month())[..3].to_uppercase();
        format!("{month} {}", at.day())
    } else {
        format!("{}/{}/{}", at.month(), at.day(), at.year())
    }
}

fn day_group(value: Option<&str>, now: &jiff::Zoned) -> String {
    let Some(at) = zoned(value) else { return String::new() };
    let date = at.date();
    let today = now.date();
    if date == today {
        return "Today".into();
    }
    if date == today.yesterday().unwrap_or(today) {
        return "Yesterday".into();
    }
    let week_ago = today.checked_sub(7.days()).unwrap_or(today);
    if date >= week_ago {
        return "Last 7 days".into();
    }
    if date.year() == today.year() && date.month() == today.month() {
        return "Earlier this month".into();
    }
    if date.year() == today.year() {
        return month_name(date.month()).into();
    }
    format!("{} {}", month_name(date.month()), date.year())
}

fn month_name(month: i8) -> &'static str {
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ];
    MONTHS[(month.clamp(1, 12) - 1) as usize]
}

fn is_popular_domain(domain: &str) -> bool {
    matches!(
        domain,
        "gmail.com"
            | "googlemail.com"
            | "icloud.com"
            | "me.com"
            | "mac.com"
            | "outlook.com"
            | "hotmail.com"
            | "live.com"
            | "yahoo.com"
            | "aol.com"
            | "proton.me"
            | "protonmail.com"
            | "hey.com"
    )
}

// -------------------------------------------------------------------------------------------
// Small helpers and the command loop.
// -------------------------------------------------------------------------------------------

fn split_addresses(value: &str) -> Vec<String> {
    value
        .split([',', ';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

impl eframe::App for ExoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.p.install(&ctx);
        self.pump_events();

        for action in self.commands.dispatch(&ctx) {
            self.apply(action);
        }
        self.commands.set_region(if self.compose.is_some() {
            Region::ComposeForm
        } else if self.thread.is_some() {
            Region::ThreadMessages
        } else if self.view == View::Search && self.search_focus {
            Region::SearchBar
        } else {
            Region::ThreadList
        });
        self.register_commands();
        self.paint_ui(ui);

        let area = ui.max_rect();
        for action in self.commands.show(&ctx, area) {
            self.apply(action);
        }

        if let Some(error) = self.error.take() {
            self.commands.toast(Toast::new(error));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_addresses() {
        assert_eq!(
            split_addresses("a@example.com, b@example.com;\n c@example.com"),
            vec!["a@example.com", "b@example.com", "c@example.com"]
        );
        assert!(split_addresses("   ").is_empty());
    }

    #[test]
    fn groups_dates_like_exo() {
        let now: jiff::Zoned = "2026-11-22T09:00:00[America/New_York]".parse().unwrap();
        assert_eq!(day_group(Some("2026-11-22T08:00:00Z"), &now), "Today");
        assert_eq!(day_group(Some("2026-11-21T08:00:00Z"), &now), "Yesterday");
        assert_eq!(day_group(Some("2026-11-18T08:00:00Z"), &now), "Last 7 days");
        assert_eq!(day_group(Some("2026-11-10T08:00:00Z"), &now), "Earlier this month");
        assert_eq!(day_group(Some("2026-10-25T08:00:00Z"), &now), "October");
        assert_eq!(day_group(Some("2025-10-25T08:00:00Z"), &now), "October 2025");
    }

    #[test]
    fn popular_domains_are_not_links() {
        assert!(is_popular_domain("hey.com"));
        assert!(!is_popular_domain("brightmoor.co"));
    }
}
