use crate::{
    core::{self, FeedResult, Post},
    storage::Store,
};
use eframe::egui::{self, Color32, RichText, TextEdit};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

const TEAL: Color32 = Color32::from_rgb(51, 162, 166);
#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Following,
    Bookmarks,
    Search,
    Settings,
}

pub struct NosturApp {
    public_key: String,
    relays: String,
    account: String,
    tab: Tab,
    feed: Vec<Post>,
    bookmarks: Vec<Post>,
    search: String,
    state: String,
    pending: Option<Receiver<Result<FeedResult, String>>>,
    store: Option<Store>,
}
impl NosturApp {
    pub fn new(cc: &eframe::CreationContext<'_>, data_dir: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let store = data_dir.and_then(|p| {
            std::fs::create_dir_all(&p)
                .ok()
                .and_then(|_| Store::open(&p.join("nostur-public.sqlite3")).ok())
        });
        let config = store.as_ref().and_then(|s| s.read_config().ok().flatten());
        let (public_key, relays) = config.unwrap_or_default();
        let account = core::parse_public_key(&public_key)
            .map(|p| p.to_hex())
            .unwrap_or_default();
        let feed = store
            .as_ref()
            .and_then(|s| s.cached_feed(&account).ok())
            .unwrap_or_default();
        let bookmarks = store
            .as_ref()
            .and_then(|s| s.bookmarks(&account).ok())
            .unwrap_or_default();
        let state = if store.is_none() {
            "Local database unavailable; this session will not persist."
        } else if !feed.is_empty() {
            "Offline: displaying cached public posts. Tap Refresh to connect."
        } else {
            "Offline. Set a public key and choose relays before connecting."
        };
        let tab = if account.is_empty() {
            Tab::Settings
        } else {
            Tab::Following
        };
        Self {
            public_key,
            relays,
            account,
            tab,
            feed,
            bookmarks,
            search: String::new(),
            state: state.into(),
            pending: None,
            store,
        }
    }
    fn refresh(&mut self, ctx: &egui::Context) {
        let key = match core::parse_public_key(&self.public_key) {
            Ok(v) => v,
            Err(e) => {
                self.state = e.to_string();
                return;
            }
        };
        let relays = match core::parse_relays(&self.relays) {
            Ok(v) => v,
            Err(e) => {
                self.state = e.to_string();
                return;
            }
        };
        let next = key.to_hex();
        if self.account != next {
            self.feed.clear();
            self.bookmarks.clear();
        }
        self.account = next;
        if let Some(s) = &mut self.store {
            if let Err(e) = s.save_config(&self.public_key, &relays.join("\n")) {
                self.state = format!("Could not save settings: {e}");
                return;
            }
            self.bookmarks = s.bookmarks(&self.account).unwrap_or_default();
            if self.feed.is_empty() {
                self.feed = s.cached_feed(&self.account).unwrap_or_default();
            }
        }
        self.state = "Connecting only to your selected relays…".into();
        self.tab = Tab::Following;
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
                .and_then(|rt| {
                    rt.block_on(core::fetch_following(key, relays))
                        .map_err(|e| e.to_string())
                });
            let _ = tx.send(result);
            repaint.request_repaint();
        });
    }
    fn accept_result(&mut self) {
        let Some(rx) = &self.pending else { return };
        match rx.try_recv() {
            Ok(result) => {
                self.pending = None;
                match result {
                    Ok(result) => {
                        if !result.follow_list_found {
                            self.state="No verified follow list found on these relays. Showing any cached posts; results may be incomplete.".into();
                        } else {
                            self.state = if result.truncated_follows {
                                format!(
                                    "Showing up to 2,000 of {} follows. Results may be incomplete.",
                                    result.follow_count
                                )
                            } else {
                                format!(
                                    "{} follows. Relay results may be incomplete.",
                                    result.follow_count
                                )
                            };
                            if let Some(s) = &mut self.store {
                                if let Err(e) = s.replace_feed(&self.account, &result.posts) {
                                    self.state =
                                        format!("Loaded, but failed to save the cache: {e}");
                                }
                            }
                            self.feed = result.posts;
                        }
                    }
                    Err(e) => self.state = format!("Refresh failed; showing any cached posts. {e}"),
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.state = "Network worker stopped. Try refreshing again.".into();
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    fn toggle_bookmark(&mut self, post: &Post) {
        let enabled = !self.bookmarks.iter().any(|p| p.id == post.id);
        match &mut self.store {
            Some(store) => match store.set_bookmark(&self.account, post, enabled) {
                Ok(()) => {
                    self.bookmarks = store.bookmarks(&self.account).unwrap_or_default();
                }
                Err(e) => self.state = format!("Could not save bookmark: {e}"),
            },
            None => self.state = "Local database unavailable; bookmark not saved.".into(),
        }
    }
}
impl eframe::App for NosturApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.accept_result();
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").color(TEAL).size(20.));
            ui.add_space(8.);
            ui.heading(match self.tab {
                Tab::Following => "Following",
                Tab::Bookmarks => "Bookmarks",
                Tab::Search => "Search",
                Tab::Settings => "Account & relays",
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.tab == Tab::Following
                    && ui
                        .add_enabled(self.pending.is_none(), egui::Button::new("Refresh"))
                        .clicked()
                {
                    self.refresh(&ctx);
                }
            });
        });
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            for (label, tab) in [
                ("Following", Tab::Following),
                ("Bookmarks", Tab::Bookmarks),
                ("Search", Tab::Search),
                ("Settings", Tab::Settings),
            ] {
                if ui.selectable_label(self.tab == tab, label).clicked() {
                    self.tab = tab;
                }
            }
        });
        ui.separator();
        ui.label(
            RichText::new(&self.state)
                .color(Color32::LIGHT_GRAY)
                .size(12.),
        );
        if self.tab == Tab::Settings {
            ui.add_space(8.);
            ui.label("Public key (npub or hex). Never enter a private key.");
            if ui
                .add_enabled(
                    self.pending.is_none(),
                    TextEdit::singleline(&mut self.public_key)
                        .desired_width(f32::INFINITY)
                        .hint_text("npub…"),
                )
                .changed()
            {
                self.feed.clear();
                self.bookmarks.clear();
                self.state = "Account changed; connect to load its feed.".into();
            }
            ui.add_space(8.);
            ui.label("Secure relay URLs, one per line");
            if ui
                .add_enabled(
                    self.pending.is_none(),
                    TextEdit::multiline(&mut self.relays)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .hint_text("wss://your-relay.example"),
                )
                .changed()
            {
                self.feed.clear();
                self.state = "Relay selection changed; connect to refresh.".into();
            }
            ui.add_space(5.);
            ui.small("No automatic connections or default relays. Selected relays can see your IP, public key and requests. Images and links are not loaded.");
            if ui
                .add_enabled(
                    self.pending.is_none(),
                    egui::Button::new("Save and connect"),
                )
                .clicked()
            {
                self.refresh(&ctx);
            }
            ui.add_space(8.);
            ui.small("Development build: read-only. Posting, DMs, payments and media are not implemented.");
            return;
        }
        if self.tab == Tab::Search {
            ui.add_space(6.);
            ui.add(
                TextEdit::singleline(&mut self.search)
                    .desired_width(f32::INFINITY)
                    .hint_text("Search cached public posts"),
            );
            ui.small("Search is local to the currently cached feed.");
        }
        let search = self.search.to_lowercase();
        let posts = if self.tab == Tab::Bookmarks {
            self.bookmarks.clone()
        } else {
            self.feed.clone()
        };
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut count = 0;
                for post in &posts {
                    if self.tab == Tab::Search
                        && !post.content.to_lowercase().contains(&search)
                        && !post.author.contains(&search)
                    {
                        continue;
                    }
                    count += 1;
                    ui.add_space(8.);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").color(TEAL).size(20.));
                        ui.label(RichText::new(&post.author[..12.min(post.author.len())]).strong());
                        ui.label(
                            RichText::new(format!("  {}", post.created_at))
                                .color(Color32::GRAY)
                                .size(11.),
                        );
                    });
                    ui.label(&post.content);
                    let marked = self.bookmarks.iter().any(|p| p.id == post.id);
                    if ui
                        .button(if marked { "★ Saved" } else { "☆ Save" })
                        .clicked()
                    {
                        clicked = Some(post.clone());
                    }
                    ui.add_space(4.);
                    ui.separator();
                }
                if count == 0 {
                    ui.add_space(14.);
                    ui.label(match self.tab {
                        Tab::Bookmarks => "No saved posts for this account.",
                        Tab::Search => "No matching cached posts.",
                        _ => "No posts loaded. Configure your account and relays in Settings.",
                    });
                }
            });
        if let Some(post) = clicked {
            self.toggle_bookmark(&post);
        }
    }
}
