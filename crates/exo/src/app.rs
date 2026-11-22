//! The three-pane HEY client and its keyboard layer.

use std::collections::HashMap;
use tokio::sync::mpsc;

use egui::{Align, Align2, Color32, FontId, Layout, RichText, ScrollArea, Sense, Vec2};
use exo_commands::{Command, Commands, Region, Toast};
use exo_store::{
    BoxInfo, Identity, Label, ScreenerItem, SearchHit, Thread, ThreadRow,
};
use hey_sdk::{Config as HeyConfig, services::BoxKind};

use crate::{
    backend::{Command as BackendCommand, Event as BackendEvent},
    body::BodyRenderer,
    theme::Palette,
};

const BOX_ORDER: &[&str] = &["imbox", "feedbox", "laterbox", "asidebox", "trailbox", "bubblebox"];
const MAIL: &[Region] = &[Region::ThreadList, Region::ThreadMessages];
const COMPOSE_REGIONS: &[Region] = &[Region::ComposeForm];

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

    fn unread(&self, kind: &str) -> usize {
        self.rows
            .get(kind)
            .map(|rows| rows.iter().filter(|r| !r.seen).count())
            .unwrap_or_default()
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
        self.send(BackendCommand::LoadBox(kind.to_string()));
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Next => {
                let len = self.current_rows().len();
                if len > 0 {
                    self.selected = Some(self.selected.map_or(0, |i| (i + 1).min(len - 1)));
                    self.open_selected();
                }
            }
            Action::Prev => {
                let len = self.current_rows().len();
                if len > 0 {
                    self.selected = Some(self.selected.map_or(0, |i| i.saturating_sub(1)));
                    self.open_selected();
                }
            }
            Action::Open => self.open_selected(),
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
                self.reply_open = true;
                self.reply_text.clear();
            }
            Action::Compose => {
                self.compose = Some(ComposeDraft::default());
                self.compose_focus = true;
            }
            Action::Refresh => {
                self.starting = true;
                self.send(BackendCommand::Refresh);
            }
            Action::Search => self.search_focus = true,
            Action::Screener => {
                self.view = View::Screener;
                self.thread = None;
            }
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
                self.send(BackendCommand::SignOut);
            }
            Action::Send => self.send_compose(false),
            Action::SaveDraft => self.send_compose(true),
            Action::Cancel => {
                self.compose = None;
                self.reply_open = false;
                self.commands.close_palette();
            }
        }
    }

    fn open_selected(&mut self) {
        if let Some(row) = self.current_posting().cloned() {
            if row.topic_id != 0 {
                self.thread = None;
                self.send(BackendCommand::LoadThread(row.topic_id));
                self.status = format!("Opening {}", row.subject);
            } else {
                self.status = "This bundle has no single thread to open".into();
            }
        }
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
                }
                BackendEvent::BoxLoaded { kind, rows } => {
                    self.rows.insert(kind, rows);
                }
                BackendEvent::BoxesUpdated { rows } => {
                    self.rows = rows;
                }
                BackendEvent::ThreadLoaded { thread } => {
                    self.thread = Some(thread);
                    self.reply_open = false;
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

    fn register_commands(&mut self) {
        let c = &mut self.commands;
        c.add(Command::shortcut("NEXT", Action::Next).keys(["j", "down"]).regions(MAIL));
        c.add(Command::shortcut("PREV", Action::Prev).keys(["k", "up"]).regions(MAIL));
        c.add(Command::new("OPEN", "Open thread", Action::Open).key("enter").regions(MAIL));
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

    fn paint_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        if !self.signed_in {
            egui::CentralPanel::default_margins().show(ui, |ui| self.sign_in_ui(ui));
            return;
        }

        egui::Panel::top("top")
            .default_size(58.0)
            .min_size(58.0)
            .max_size(58.0)
            .resizable(false)
            .show(ui, |ui| self.top_bar(ui));

        egui::Panel::left("rail")
            .default_size(58.0)
            .min_size(58.0)
            .max_size(58.0)
            .resizable(false)
            .show(ui, |ui| self.rail(ui));

        egui::Panel::left("sidebar")
            .default_size(212.0)
            .min_size(212.0)
            .max_size(212.0)
            .resizable(false)
            .show(ui, |ui| self.sidebar(ui));

        match self.view {
            View::Screener => {
                egui::CentralPanel::default_margins()
                    .frame(egui::Frame::new().fill(self.p.window))
                    .show(ui, |ui| self.screener_ui(ui));
            }
            View::Search => {
                egui::CentralPanel::default_margins()
                    .frame(egui::Frame::new().fill(self.p.window))
                    .show(ui, |ui| self.search_ui(ui));
            }
            View::Mail => {
                egui::Panel::left("list")
                    .default_size(600.0)
                    .min_size(430.0)
                    .resizable(true)
                    .show(ui, |ui| self.list_ui(ui));
                egui::CentralPanel::default_margins()
                    .frame(egui::Frame::new().fill(self.p.window))
                    .show(ui, |ui| self.reader_ui(ui));
            }
        }

        if let Some(compose) = self.compose.clone() {
            let mut open = true;
            egui::Window::new("New message")
                .open(&mut open)
                .default_width(660.0)
                .default_height(520.0)
                .resizable(true)
                .show(&ctx, |ui| self.compose_ui(ui, compose));
            if !open {
                self.compose = None;
            }
        }
    }

    fn sign_in_ui(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 130.0).max(40.0));
            ui.label(RichText::new("EXO").size(13.0).strong().color(self.p.faint));
            ui.add_space(10.0);
            ui.label(RichText::new("Your mail, at the speed of thought.").size(34.0).strong().color(self.p.ink));
            ui.add_space(10.0);
            ui.label(RichText::new("A native client for HEY — local-first, keyboard-driven, and quietly fast.").size(14.0).color(self.p.muted));
            ui.add_space(32.0);
            let button = egui::Button::new(
                RichText::new(if self.starting { "Waiting for HEY…" } else { "Sign in with HEY" })
                    .size(15.0)
                    .strong()
                    .color(self.p.accent_ink),
            )
            .fill(self.p.accent)
            .min_size(Vec2::new(240.0, 46.0));
            if ui.add(button).clicked() {
                self.starting = true;
                self.send(BackendCommand::SignIn);
            }
            ui.add_space(16.0);
            ui.label(RichText::new("Your browser opens to approve access. Tokens stay on this device.").size(12.0).color(self.p.faint));
        });
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.set_min_height(56.0);
        egui::Frame::new().fill(self.p.panel).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("EXO").size(12.0).strong().color(self.p.muted));
                ui.label(RichText::new("/").size(12.0).color(self.p.faint));
                let title = if self.view == View::Mail {
                    self.box_name(&self.selected_box)
                } else if self.view == View::Screener {
                    "The Screener".to_string()
                } else {
                    "Search".to_string()
                };
                ui.label(RichText::new(title).size(14.0).strong().color(self.p.ink));
                if self.view == View::Mail && self.unread(&self.selected_box) > 0 {
                    ui.label(
                        RichText::new(self.unread(&self.selected_box).to_string())
                            .size(11.0)
                            .strong()
                            .color(self.p.muted),
                    );
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(20.0);
                    let name = if self.identity.name.is_empty() {
                        self.identity.email.clone()
                    } else {
                        self.identity.name.clone()
                    };
                    let account_initials = initials(&name);
                    ui.label(RichText::new(name).size(12.0).color(self.p.muted));
                    ui_avatar(ui, &account_initials, "", self.p, 26.0);
                    ui.add_space(14.0);
                    if ui.button(RichText::new("Compose").size(12.0).color(self.p.ink)).clicked() {
                        self.compose = Some(ComposeDraft::default());
                        self.compose_focus = true;
                    }
                    if ui.button(RichText::new("Search").size(12.0).color(self.p.ink)).clicked() {
                        self.search_focus = true;
                        self.commands.open_palette(None);
                    }
                    ui.add_space(6.0);
                });
            });
        });
        ui.add(egui::Separator::default().spacing(0.0));
    }

    fn rail(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(self.p.panel).show(ui, |ui| {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                for (kind, glyph) in [
                    ("imbox", "I"),
                    ("feedbox", "F"),
                    ("laterbox", "L"),
                    ("asidebox", "A"),
                    ("trailbox", "T"),
                ] {
                    let active = self.view == View::Mail && self.selected_box == kind;
                    let size = 38.0;
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
                    if active {
                        ui.painter()
                            .rect_filled(rect, 9.0, self.p.selected);
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, rect.height())),
                            0.0,
                            Color32::from_rgb(64, 132, 226),
                        );
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, 9.0, self.p.hover);
                    }
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        glyph,
                        FontId::proportional(17.0),
                        if active { self.p.ink } else { self.p.muted },
                    );
                    let response = response.on_hover_text(self.box_name(kind));
                    if response.clicked() {
                        self.select_box(kind);
                    }
                    ui.add_space(7.0);
                }
                ui.add_space(8.0);
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::click());
                if self.view == View::Screener {
                    ui.painter().rect_filled(rect, 9.0, self.p.selected);
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 9.0, self.p.hover);
                }
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "S",
                    FontId::proportional(17.0),
                    if self.view == View::Screener { self.p.ink } else { self.p.muted },
                );
                if response.clicked() {
                    self.view = View::Screener;
                }
            });
        });
        ui.add(egui::Separator::default().spacing(0.0));
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(self.p.panel).show(ui, |ui| {
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.label(RichText::new("MAILBOXES").size(10.0).strong().color(self.p.faint));
            });
            ui.add_space(8.0);
            for kind in BOX_ORDER {
                self.box_button(ui, kind);
            }

            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.label(RichText::new("LABELS").size(10.0).strong().color(self.p.faint));
            });
            ui.add_space(6.0);
            for label in &self.labels {
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width() - 14.0, 28.0),
                    Sense::click(),
                );
                if response.hovered() {
                    ui.painter().rect_filled(rect, 6.0, self.p.hover);
                }
                ui.painter().circle_filled(
                    egui::pos2(rect.left() + 13.0, rect.center().y),
                    3.5,
                    Color32::from_rgb(126, 176, 132),
                );
                ui.painter().text(
                    egui::pos2(rect.left() + 26.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    &label.name,
                    FontId::proportional(12.5),
                    self.p.ink,
                );
                if response.clicked() {
                    self.status = format!("Select a thread, then use ⌘K to apply “{}”", label.name);
                }
            }
        });

        ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
            egui::Frame::new().fill(self.p.panel).show(ui, |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    if ui.add(egui::Button::new(RichText::new("Compose").size(12.5).color(self.p.muted)).frame(false)).clicked() {
                        self.compose = Some(ComposeDraft::default());
                    }
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    if ui.add(egui::Button::new(RichText::new("Refresh").size(12.5).color(self.p.muted)).frame(false)).clicked() {
                        self.send(BackendCommand::Refresh);
                    }
                    ui.add_space(8.0);
                    if ui.add(egui::Button::new(RichText::new("Sign out").size(11.5).color(self.p.faint)).frame(false)).clicked() {
                        self.send(BackendCommand::SignOut);
                        self.signed_in = false;
                    }
                });
                ui.add_space(10.0);
            });
        });
        ui.add(egui::Separator::default().spacing(0.0));
    }

    fn box_button(&mut self, ui: &mut egui::Ui, kind: &str) {
        let active = self.view == View::Mail && self.selected_box == kind;
        let unread = self.unread(kind);
        let name = self.box_name(kind);
        let height = 32.0;
        let rect = ui.available_rect_before_wrap();
        let rect = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), height));
        let response = ui.interact(rect, egui::Id::new(("box", kind)), Sense::click());
        if active {
            ui.painter().rect_filled(rect, 6.0, self.p.selected);
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, rect.height())),
                0.0,
                Color32::from_rgb(64, 132, 226),
            );
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 6.0, self.p.hover);
        }
        let x = rect.left() + 18.0;
        ui.painter().text(
            egui::pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            &name,
            FontId::proportional(12.8),
            if active || unread > 0 { self.p.ink } else { self.p.muted },
        );
        if unread > 0 {
            let label = unread.to_string();
            ui.painter().text(
                egui::pos2(rect.right() - 16.0, rect.center().y),
                Align2::RIGHT_CENTER,
                &label,
                FontId::proportional(11.0),
                if active { self.p.ink } else { self.p.muted },
            );
        }
        ui.allocate_rect(rect, Sense::hover());
        if response.clicked() {
            self.select_box(kind);
        }
    }

    fn list_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(self.p.list).show(ui, |ui| {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new(self.box_name(&self.selected_box)).size(20.0).strong().color(self.p.ink));
                ui.add_space(6.0);
                ui.label(RichText::new(self.current_rows().len().to_string()).size(12.0).color(self.p.faint));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(20.0);
                    if ui.button(RichText::new("⌕ Search").size(11.5).color(self.p.muted)).clicked() {
                        self.search_focus = true;
                    }
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                let search = egui::TextEdit::singleline(&mut self.search_text)
                    .hint_text("Search all mail")
                    .frame(egui::Frame::NONE)
                    .desired_width(ui.available_width() - 40.0);
                let response = ui.add(search);
                if self.search_focus {
                    response.request_focus();
                    self.search_focus = false;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let query = self.search_text.clone();
                    self.send(BackendCommand::Search(query));
                }
            });
            ui.add_space(8.0);
            ui.add(egui::Separator::default().spacing(0.0));
        });

        let rows = self.current_rows().to_vec();
        let mut last_group = String::new();
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (index, row) in rows.iter().enumerate() {
                let group = group_label(row.created_at.as_deref());
                if group != last_group {
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.label(RichText::new(&group).size(10.5).strong().color(self.p.faint));
                    });
                    ui.add_space(4.0);
                    last_group = group;
                }

                let height = 46.0;
                let full = ui.available_rect_before_wrap();
                let rect = egui::Rect::from_min_size(full.min, Vec2::new(full.width(), height));
                let response = ui.interact(
                    rect,
                    egui::Id::new(("row", &self.selected_box, row.posting_id)),
                    Sense::click(),
                );
                let selected = self.selected == Some(index);
                if selected {
                    ui.painter().rect_filled(rect, 0.0, self.p.selected);
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, height)),
                        0.0,
                        Color32::from_rgb(64, 132, 226),
                    );
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 0.0, self.p.hover);
                }

                let mut x = rect.left() + 14.0;
                if !row.seen {
                    ui.painter().circle_filled(
                        egui::pos2(x + 3.0, rect.center().y),
                        3.5,
                        Color32::from_rgb(64, 132, 226),
                    );
                }
                x += 14.0;

                let sender_width = 132.0;
                ui.painter().text(
                    egui::pos2(x, rect.center().y),
                    Align2::LEFT_CENTER,
                    clip(&row.display_name(), 22),
                    FontId::proportional(12.6),
                    if row.seen { self.p.muted } else { self.p.ink },
                );
                x += sender_width;

                for label_id in &row.label_ids {
                    let name = self
                        .labels
                        .iter()
                        .find(|l| l.id == *label_id)
                        .map(|l| l.name.clone())
                        .unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    let chip = egui::Rect::from_min_size(
                        egui::pos2(x, rect.center().y - 9.0),
                        Vec2::new((name.chars().count().min(14) as f32) * 6.4 + 12.0, 18.0),
                    );
                    ui.painter().rect_filled(chip, 4.0, Color32::from_rgb(226, 236, 226));
                    ui.painter().text(
                        chip.center(),
                        Align2::CENTER_CENTER,
                        clip(&name, 14),
                        FontId::proportional(9.5),
                        Color32::from_rgb(64, 104, 70),
                    );
                    x += chip.width() + 5.0;
                }

                ui.painter().text(
                    egui::pos2(x, rect.center().y - 8.0),
                    Align2::LEFT_CENTER,
                    clip(&row.subject, 40),
                    FontId::proportional(12.4),
                    if row.seen { self.p.muted } else { self.p.ink },
                );
                ui.painter().text(
                    egui::pos2(x, rect.center().y + 9.0),
                    Align2::LEFT_CENTER,
                    clip(&row.summary, 58),
                    FontId::proportional(11.0),
                    self.p.faint,
                );

                let date = short_date(row.created_at.as_deref());
                ui.painter().text(
                    egui::pos2(rect.right() - 14.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    &date,
                    FontId::proportional(10.5),
                    self.p.faint,
                );

                ui.add_space(height);
                ui.add(egui::Separator::default().spacing(0.0));

                if response.clicked() {
                    self.selected = Some(index);
                    self.open_selected();
                }
            }
            if rows.is_empty() {
                ui.add_space(100.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Nothing here yet").size(16.0).color(self.p.faint));
                    ui.add_space(6.0);
                    ui.label(RichText::new("New mail will show up here as it arrives.").size(12.0).color(self.p.faint));
                });
            }
        });
    }

    fn reader_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(self.p.window).show(ui, |ui| {
            let Some(thread) = self.thread.clone() else {
                ui.add_space(140.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Nothing selected").size(17.0).color(self.p.faint));
                    ui.add_space(8.0);
                    ui.label(RichText::new("j / k to move · Enter to open · ⌘K for everything").size(12.0).color(self.p.faint));
                });
                return;
            };

            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.label(RichText::new(clip(&thread.subject, 70)).size(20.0).strong().color(self.p.ink));
            });
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.label(
                    RichText::new(format!("{} message(s)", thread.messages.len()))
                        .size(11.0)
                        .color(self.p.faint),
                );
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                if ui.button(RichText::new("Reply").size(11.5).strong().color(self.p.accent_ink)).clicked() {
                    self.reply_open = !self.reply_open;
                }
                if ui.button(RichText::new("Set Aside").size(11.5).color(self.p.ink)).clicked() {
                    if let Some(row) = self.current_posting() {
                        self.send(BackendCommand::Move(BoxKind::SetAside, vec![row.posting_id]));
                        self.selected = None;
                        self.thread = None;
                    }
                }
                if ui.button(RichText::new("Trash").size(11.5).color(self.p.danger)).clicked() {
                    if let Some(row) = self.current_posting() {
                        self.send(BackendCommand::Trash(vec![row.posting_id]));
                        self.selected = None;
                        self.thread = None;
                    }
                }
            });
            ui.add_space(12.0);

            let width = ui.available_width().min(760.0) - 48.0;
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (index, message) in thread.messages.iter().enumerate() {
                    let frame = egui::Frame::new()
                        .fill(self.p.card)
                        .inner_margin(egui::Margin { left: 20, right: 20, top: 16, bottom: 18 })
                        .stroke(egui::Stroke::new(1.0, self.p.line));
                    frame.show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            ui_avatar(ui, &message.sender_initials, "", self.p, 30.0);
                            ui.add_space(9.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(if message.sender_name.is_empty() {
                                        &message.sender_email
                                    } else {
                                        &message.sender_name
                                    })
                                    .size(13.0)
                                    .strong()
                                    .color(self.p.ink),
                                );
                                ui.label(
                                    RichText::new(short_date(message.sent_at.as_deref()))
                                        .size(10.5)
                                        .color(self.p.faint),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(format!("{}", index + 1)).size(10.0).color(self.p.faint));
                            });
                        });
                        ui.add_space(12.0);
                        if let Some((texture, size)) = self.body.paint(
                            ui.ctx(),
                            thread.topic_id as u64 * 1000 + index as u64,
                            &message.html,
                            width,
                            self.p.dark,
                        ) {
                            let (image_rect, _) =
                                ui.allocate_exact_size(size, egui::Sense::hover());
                            ui.painter().image(
                                texture.id(),
                                image_rect,
                                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.label(RichText::new("(empty message)").italics().color(self.p.faint));
                        }
                    });
                    ui.add_space(12.0);
                }

                let reply_frame = egui::Frame::new()
                    .fill(self.p.card)
                    .inner_margin(14.0)
                    .stroke(egui::Stroke::new(1.0, self.p.line));
                reply_frame.show(ui, |ui| {
                    ui.set_width(width);
                    ui.label(RichText::new("Reply").size(12.0).strong().color(self.p.muted));
                    ui.add_space(6.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.reply_text)
                            .desired_rows(if self.reply_open { 5 } else { 2 })
                            .hint_text("Write a reply…")
                            .desired_width(width),
                    );
                    ui.add_space(8.0);
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Send reply").size(12.0).strong().color(self.p.accent_ink))
                                .fill(self.p.accent),
                        )
                        .clicked()
                        && let Some(message) = thread.messages.last()
                    {
                        let html = exo_mail::text_to_html(&self.reply_text);
                        self.send(BackendCommand::Reply {
                            entry_id: message.id,
                            body: html,
                        });
                        self.reply_text.clear();
                        self.reply_open = false;
                    }
                });
                ui.add_space(28.0);
            });
        });
    }

    fn screener_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(self.p.window).show(ui, |ui| {
            ui.add_space(24.0);
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                ui.label(RichText::new("The Screener").size(22.0).strong().color(self.p.ink));
            });
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                ui.label(RichText::new("You decide who gets into your Imbox.").size(12.5).color(self.p.muted));
            });
            ui.add_space(16.0);
            let items = self.screener.clone();
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for item in &items {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(26.0);
                        let card = egui::Frame::new()
                            .fill(self.p.card)
                            .inner_margin(16.0)
                            .stroke(egui::Stroke::new(1.0, self.p.line));
                        card.show(ui, |ui| {
                            ui.set_width(ui.available_width() - 26.0);
                            ui.horizontal(|ui| {
                                ui_avatar(ui, &initials(&item.sender_name), "", self.p, 34.0);
                                ui.add_space(12.0);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(if item.sender_name.is_empty() { &item.sender_email } else { &item.sender_name }).size(13.5).strong().color(self.p.ink));
                                    ui.label(RichText::new(&item.subject).size(12.5).color(self.p.ink));
                                    ui.label(RichText::new(&item.summary).size(11.5).color(self.p.muted));
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add(egui::Button::new(RichText::new("Turn away").size(11.5).color(self.p.danger))).clicked() {
                                        self.send(BackendCommand::Screen(item.clearance_id, false));
                                    }
                                    if ui.add(egui::Button::new(RichText::new("Let in").size(11.5).strong().color(self.p.accent_ink)).fill(self.p.accent)).clicked() {
                                        self.send(BackendCommand::Screen(item.clearance_id, true));
                                    }
                                });
                            });
                        });
                    });
                }
                if items.is_empty() {
                    ui.add_space(100.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Nobody is waiting").size(17.0).color(self.p.faint));
                        ui.add_space(6.0);
                        ui.label(RichText::new("New senders appear here before reaching your Imbox.").size(12.0).color(self.p.faint));
                    });
                }
            });
        });
    }

    fn search_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(self.p.window).show(ui, |ui| {
            ui.add_space(24.0);
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                ui.label(RichText::new("Search").size(22.0).strong().color(self.p.ink));
                ui.add_space(16.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search_text)
                        .hint_text("Search all mail…")
                        .desired_width(340.0),
                );
                if self.search_focus {
                    response.request_focus();
                    self.search_focus = false;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let query = self.search_text.clone();
                    self.send(BackendCommand::Search(query));
                }
                if ui.button("Search").clicked() {
                    let query = self.search_text.clone();
                    self.send(BackendCommand::Search(query));
                }
            });
            ui.add_space(14.0);
            if let Some(hits) = self.hits.clone() {
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for hit in &hits {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add_space(26.0);
                            let card = egui::Frame::new().fill(self.p.card).inner_margin(14.0).stroke(egui::Stroke::new(1.0, self.p.line));
                            card.show(ui, |ui| {
                                ui.set_width(ui.available_width() - 26.0);
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(&hit.subject).size(13.0).strong().color(self.p.ink));
                                        ui.label(RichText::new(format!("{} — {}", hit.sender_name, hit.summary)).size(11.5).color(self.p.muted));
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui.button("Open").clicked() {
                                            self.view = View::Mail;
                                            self.send(BackendCommand::LoadThread(hit.topic_id));
                                        }
                                    });
                                });
                            });
                        });
                    }
                    if hits.is_empty() {
                        ui.add_space(80.0);
                        ui.vertical_centered(|ui| { ui.label(RichText::new("No matches").color(self.p.faint)); });
                    }
                });
            }
        });
    }

    fn compose_ui(&mut self, ui: &mut egui::Ui, mut compose: ComposeDraft) {
        let mut send = false;
        let mut draft = false;
        egui::Frame::new().fill(self.p.panel).inner_margin(18.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("To").size(11.5).color(self.p.faint));
                ui.add(egui::TextEdit::singleline(&mut compose.to).desired_width(ui.available_width()));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Subject").size(11.5).color(self.p.faint));
                ui.add(egui::TextEdit::singleline(&mut compose.subject).desired_width(ui.available_width()));
            });
            ui.add_space(10.0);
            ui.add(egui::Separator::default().spacing(0.0));
            ui.add_space(10.0);
            ui.add(
                egui::TextEdit::multiline(&mut compose.body)
                    .desired_rows(14)
                    .hint_text("Write your message…")
                    .desired_width(f32::INFINITY)
                    .frame(egui::Frame::NONE),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(
                    egui::Button::new(RichText::new("Send").size(12.5).strong().color(self.p.accent_ink)).fill(self.p.accent).min_size(Vec2::new(90.0, 32.0)),
                ).clicked() {
                    send = true;
                }
                if ui.button(RichText::new("Save draft").size(12.0).color(self.p.ink)).clicked() {
                    draft = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(RichText::new("Discard").size(11.5).color(self.p.faint)).clicked() {
                        self.compose = None;
                    }
                });
            });
        });
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

// Small presentation helpers for the redesigned panes.
fn ui_avatar(ui: &mut egui::Ui, initials: &str, color: &str, p: Palette, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let fill = parse_color(color).unwrap_or_else(|| {
        if p.dark {
            Color32::from_rgb(70, 76, 88)
        } else {
            Color32::from_rgb(216, 224, 236)
        }
    });
    ui.painter().circle_filled(rect.center(), size / 2.0, fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initials.chars().take(2).collect::<String>(),
        FontId::proportional(size * 0.36),
        if p.dark { Color32::WHITE } else { Color32::from_rgb(48, 62, 82) },
    );
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn today_ymd() -> (i64, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = secs.div_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += if m <= 2 { 1 } else { 0 };
    (y, m as u32, d as u32)
}

fn ymd_to_days(y: i64, m: u32, d: u32) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = m as i64 + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn group_label(iso: Option<&str>) -> String {
    let Some(iso) = iso else { return "Undated".into() };
    if iso.len() < 10 { return "Undated".into() }
    let (y, m, d) = (
        iso[0..4].parse::<i64>().unwrap_or(1970),
        iso[5..7].parse::<u32>().unwrap_or(1),
        iso[8..10].parse::<u32>().unwrap_or(1),
    );
    let (ty, tm, td) = today_ymd();
    let diff = ymd_to_days(ty, tm, td) - ymd_to_days(y, m, d);
    match diff {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=7 => "This week".into(),
        8..=31 => "This month".into(),
        _ => format!("{y}"),
    }
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
        } else if self.search_focus {
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

fn parse_color(value: &str) -> Option<Color32> {
    let hex = value.trim_start_matches('#');
    if hex.len() != 6 { return None; }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase()
}

fn split_addresses(value: &str) -> Vec<String> {
    value
        .split([',', ';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn short_date(value: Option<&str>) -> String {
    let Some(value) = value else { return String::new() };
    if value.len() >= 16 {
        // "2026-11-22T09:30:00Z" -> "Nov 22  09:30"
        let month = match &value[5..7] {
            "01" => "Jan", "02" => "Feb", "03" => "Mar", "04" => "Apr", "05" => "May", "06" => "Jun",
            "07" => "Jul", "08" => "Aug", "09" => "Sep", "10" => "Oct", "11" => "Nov", "12" => "Dec",
            _ => "???",
        };
        format!("{} {}  {}", month, &value[8..10], &value[11..16])
    } else {
        value.to_string()
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
    fn formats_hey_timestamps() {
        assert_eq!(short_date(Some("2026-11-22T09:30:00Z")), "Nov 22  09:30");
        assert_eq!(short_date(None), "");
    }

    #[test]
    fn makes_initials() {
        assert_eq!(initials("Ann Lee"), "AL");
        assert_eq!(initials(""), "");
    }
}
