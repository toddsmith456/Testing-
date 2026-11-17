use crate::{
    core::{self, FeedResult, Post, Profile},
    publish::{self, PublishReport},
    storage::{QueuedPost, Store},
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
    Compose,
    Profile,
    PostDetail,
    Settings,
}

pub struct NosturApp {
    public_key: String,
    relays: String,
    account: String,
    tab: Tab,
    feed: Vec<Post>,
    bookmarks: Vec<Post>,
    profiles: Vec<Profile>,
    selected_author: String,
    selected_post: Option<Post>,
    search: String,
    state: String,
    pending: Option<Receiver<Result<FeedResult, String>>>,
    store: Option<Store>,
    draft: String,
    bunker_uri: String,
    outbox: Vec<QueuedPost>,
    pending_sign: Option<Receiver<Result<String, String>>>,
    pending_send: Option<(String, Receiver<Result<PublishReport, String>>)>,
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
        let profiles = store
            .as_ref()
            .and_then(|s| s.profiles(&account).ok())
            .unwrap_or_default();
        let outbox = store
            .as_ref()
            .and_then(|s| s.outbox(&account).ok())
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
            profiles,
            selected_author: String::new(),
            selected_post: None,
            search: String::new(),
            state: state.into(),
            pending: None,
            store,
            draft: String::new(),
            bunker_uri: String::new(),
            outbox,
            pending_sign: None,
            pending_send: None,
        }
    }
    fn busy(&self) -> bool {
        self.pending.is_some() || self.pending_sign.is_some() || self.pending_send.is_some()
    }
    fn reload_outbox(&mut self) {
        self.outbox = self
            .store
            .as_ref()
            .and_then(|s| s.outbox(&self.account).ok())
            .unwrap_or_default();
    }
    fn active_key(&self) -> Result<nostr_sdk::prelude::PublicKey, String> {
        let key = core::parse_public_key(&self.public_key).map_err(|e| e.to_string())?;
        if key.to_hex() != self.account {
            return Err("Save and connect this account before composing or publishing.".into());
        }
        Ok(key)
    }
    fn start_sign(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let key = match self.active_key() {
            Ok(k) => k,
            Err(e) => {
                self.state = e;
                return;
            }
        };
        if self.store.is_none() {
            self.state = "Local outbox unavailable; cannot sign safely.".into();
            return;
        }
        if let Err(e) = publish::signer_relays(&self.bunker_uri) {
            self.state = e.to_string();
            return;
        }
        if let Err(e) = publish::build_note(key, &self.draft) {
            self.state = e.to_string();
            return;
        }
        let uri = self.bunker_uri.clone();
        let content = self.draft.clone();
        let (tx, rx) = mpsc::channel();
        self.pending_sign = Some(rx);
        self.state = "Waiting for the remote signer. The post has not been published.".into();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|_| "Could not start signer worker".to_string())
                .and_then(|rt| {
                    rt.block_on(publish::sign_remote(&uri, key, &content))
                        .map_err(|e| e.to_string())
                });
            let _ = tx.send(result);
            repaint.request_repaint();
        });
    }
    fn start_publish(&mut self, ctx: &egui::Context, item: QueuedPost) {
        if self.busy() {
            return;
        }
        let key = match self.active_key() {
            Ok(k) => k,
            Err(e) => {
                self.state = e;
                return;
            }
        };
        let relays = match core::parse_relays(&self.relays) {
            Ok(r) => r,
            Err(e) => {
                self.state = e.to_string();
                return;
            }
        };
        if publish::checked_event(key, &item.event_json).is_err() {
            self.state = "Queued event failed verification.".into();
            return;
        }
        let Some(store) = &mut self.store else {
            self.state = "Local outbox unavailable.".into();
            return;
        };
        if let Err(e) = store.update_outbox(
            &self.account,
            &item.id,
            "sending",
            "If interrupted, delivery is unknown.",
        ) {
            self.state = format!("Cannot update outbox: {e}");
            return;
        }
        self.reload_outbox();
        let id = item.id.clone();
        let json = item.event_json.clone();
        let (tx, rx) = mpsc::channel();
        self.pending_send = Some((id, rx));
        self.state = "Sending public event to selected relays…".into();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|_| "Cannot start publishing worker".to_string())
                .and_then(|rt| {
                    rt.block_on(publish::publish_queued(key, &json, relays))
                        .map_err(|e| e.to_string())
                });
            let _ = tx.send(result);
            repaint.request_repaint();
        });
    }
    fn accept_post_result(&mut self) {
        if let Some(rx) = &self.pending_sign {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending_sign = None;
                    self.bunker_uri.clear();
                    match result {
                        Ok(json) => {
                            if let Some(store) = &mut self.store {
                                match store.enqueue_signed(&self.account, &json) {
                                    Ok(_) => {
                                        self.draft.clear();
                                        self.state =
                                            "Signed and queued locally. Not published yet.".into();
                                    }
                                    Err(e) => {
                                        self.state = format!(
                                            "Signed event could not be saved; not published: {e}"
                                        )
                                    }
                                }
                            }
                            self.reload_outbox();
                        }
                        Err(e) => self.state = format!("Signing failed; nothing published. {e}"),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending_sign = None;
                    self.bunker_uri.clear();
                    self.state = "Signer worker stopped; nothing published.".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some((id, rx)) = &self.pending_send {
            let result = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Publish worker stopped; delivery may be unknown.".into(),
                )),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let id = id.clone();
                self.pending_send = None;
                let (status, details) = match result {
                    Ok(r) => {
                        let message = format!(
                            "Accepted: {}. Not confirmed: {}.",
                            if r.accepted.is_empty() {
                                "none".into()
                            } else {
                                r.accepted.join(", ")
                            },
                            if r.not_confirmed.is_empty() {
                                "none".into()
                            } else {
                                r.not_confirmed.join(", ")
                            }
                        );
                        (
                            if r.not_confirmed.is_empty() {
                                "accepted"
                            } else {
                                "partial / unknown"
                            },
                            message,
                        )
                    }
                    Err(e) => (
                        "unknown",
                        format!("{e}. Delivery may be unknown; inspect relays before retrying."),
                    ),
                };
                if let Some(store) = &mut self.store {
                    if let Err(e) = store.update_outbox(&self.account, &id, status, &details) {
                        self.state =
                            format!("Could not record relay result; delivery unknown: {e}");
                        return;
                    }
                }
                self.state =
                    format!("{status}: {details} An accepted relay does not guarantee delivery.");
                self.reload_outbox();
            }
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
            self.profiles.clear();
        }
        self.account = next;
        self.reload_outbox();
        if let Some(s) = &mut self.store {
            if let Err(e) = s.save_config(&self.public_key, &relays.join("\n")) {
                self.state = format!("Could not save settings: {e}");
                return;
            }
            self.bookmarks = s.bookmarks(&self.account).unwrap_or_default();
            self.profiles = s.profiles(&self.account).unwrap_or_default();
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
                                if let Err(e) = s.merge_profiles(&self.account, &result.profiles) {
                                    self.state = format!("Profile cache error: {e}");
                                }
                                self.profiles = s.profiles(&self.account).unwrap_or_default();
                                if let Err(e) = s.replace_feed(&self.account, &result.posts) {
                                    self.state =
                                        format!("Loaded, but failed to save the cache: {e}");
                                }
                            }
                            self.feed = self
                                .store
                                .as_ref()
                                .and_then(|s| s.cached_feed(&self.account).ok())
                                .unwrap_or(result.posts);
                            self.state
                                .push_str(" Cached posts may include older or unfollowed authors.");
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
        self.accept_post_result();
        let busy = self.busy();
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").color(TEAL).size(20.));
            ui.add_space(8.);
            ui.heading(match self.tab {
                Tab::Following => "Following",
                Tab::Bookmarks => "Bookmarks",
                Tab::Search => "Search",
                Tab::Compose => "New post",
                Tab::Profile => "Profile",
                Tab::PostDetail => "Post",
                Tab::Settings => "Account & relays",
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.tab == Tab::Following
                    && ui
                        .add_enabled(!busy, egui::Button::new("Refresh"))
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
                ("New post", Tab::Compose),
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
                    !busy,
                    TextEdit::singleline(&mut self.public_key)
                        .desired_width(f32::INFINITY)
                        .hint_text("npub…"),
                )
                .changed()
            {
                self.feed.clear();
                self.bookmarks.clear();
                self.profiles.clear();
                self.state = "Account changed; connect to load its feed.".into();
            }
            ui.add_space(8.);
            ui.label("Secure relay URLs, one per line");
            if ui
                .add_enabled(
                    !busy,
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
                .add_enabled(!busy, egui::Button::new("Save and connect"))
                .clicked()
            {
                self.refresh(&ctx);
            }
            ui.add_space(8.);
            ui.small("Development build. NIP-46 signing is experimental; DMs, payments and media are not implemented.");
            return;
        }
        if self.tab == Tab::Compose {
            ui.add_space(6.);
            ui.label(RichText::new("Public post").strong());
            ui.small(format!(
                "Selected account: {}",
                if self.account.is_empty() {
                    "none"
                } else {
                    &self.account
                }
            ));
            ui.add_enabled(
                !busy,
                TextEdit::multiline(&mut self.draft)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .hint_text("Write a public note…"),
            );
            ui.small("Text is public if published. Draft is in memory only until signed; it is lost if the app closes.");
            ui.label("NIP-46 bunker URI (may contain a secret; never share it)");
            ui.add_enabled(
                !busy,
                TextEdit::singleline(&mut self.bunker_uri)
                    .password(true)
                    .desired_width(f32::INFINITY)
                    .hint_text("bunker://…"),
            );
            if !self.bunker_uri.is_empty() {
                match publish::signer_relays(&self.bunker_uri) {
                    Ok(relays) => {
                        ui.small(format!("Signer relay contacts: {}", relays.join(", ")));
                    }
                    Err(_) => {
                        ui.small("Enter a valid bunker URI with secure relay addresses.");
                    }
                }
            }
            ui.small("The signer sees your request; its relays see connection metadata. The URI and temporary client key are not saved.");
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new("Request signature (do not publish yet)"),
                )
                .clicked()
            {
                self.start_sign(&ctx);
            }
            ui.separator();
            ui.heading("Signed outbox");
            ui.small("Publish explicitly below. Relay acceptance is not delivery. Sending may succeed even if a timeout occurs. Removing a local record does not delete published copies.");
            ui.small(format!(
                "Publish destinations: {}",
                if self.relays.trim().is_empty() {
                    "none selected"
                } else {
                    self.relays.trim()
                }
            ));
            let items = self.outbox.clone();
            let mut action = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if items.is_empty() {
                        ui.label("No signed events for this account.");
                    }
                    for item in &items {
                        if let Ok(key) = self.active_key() {
                            if let Ok(event) = publish::checked_event(key, &item.event_json) {
                                ui.label(event.content);
                            }
                        }
                        ui.small(format!(
                            "{} · {}",
                            &item.id[..12.min(item.id.len())],
                            item.status
                        ));
                        if !item.details.is_empty() {
                            ui.small(&item.details);
                        }
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(!busy, egui::Button::new("Publish / retry"))
                                .clicked()
                            {
                                action = Some((true, item.clone()));
                            }
                            if ui
                                .add_enabled(!busy, egui::Button::new("Remove local record"))
                                .clicked()
                            {
                                action = Some((false, item.clone()));
                            }
                        });
                        ui.separator();
                    }
                });
            if let Some((send, item)) = action {
                if send {
                    self.start_publish(&ctx, item);
                } else if let Some(store) = &mut self.store {
                    match store.delete_outbox(&self.account, &item.id) {
                        Ok(()) => {
                            self.state = "Local record removed; relay copies may remain.".into()
                        }
                        Err(e) => self.state = format!("Could not remove local record: {e}"),
                    };
                    self.reload_outbox();
                }
            }
            return;
        }
        if self.tab == Tab::PostDetail {
            if ui.button("← Back to Following").clicked() {
                self.tab = Tab::Following;
            }
            if let Some(post) = &self.selected_post {
                ui.add_space(10.);
                ui.label(RichText::new("Public note").strong());
                ui.label(&post.content);
                ui.separator();
                ui.small(format!("Author: {}", post.author));
                ui.small(format!("Event ID: {}", post.id));
                ui.small(format!("Published: {}", relative_time(post.created_at)));
                ui.small("Replies and reactions are not yet implemented.");
            }
            return;
        }
        if self.tab == Tab::Profile {
            if ui.button("← Back to Following").clicked() {
                self.tab = Tab::Following;
            }
            if let Some(profile) = self
                .profiles
                .iter()
                .find(|p| p.author == self.selected_author)
            {
                ui.heading(if profile.name.is_empty() {
                    &profile.author[..12]
                } else {
                    &profile.name
                });
                if !profile.about.is_empty() {
                    ui.label(&profile.about);
                }
            } else {
                ui.heading("Profile");
                ui.small("Profile metadata not available in cache.");
            }
            ui.small(&self.selected_author);
            ui.separator();
            ui.label("Cached posts");
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
        let mut open_author = None;
        let mut open_post = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut count = 0;
                for post in &posts {
                    if self.tab == Tab::Profile && post.author != self.selected_author {
                        continue;
                    }
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
                        let name = self
                            .profiles
                            .iter()
                            .find(|p| p.author == post.author && !p.name.is_empty())
                            .map(|p| p.name.as_str())
                            .unwrap_or(&post.author[..12.min(post.author.len())]);
                        if ui.button(RichText::new(name).strong()).clicked() {
                            open_author = Some(post.author.clone());
                        }
                        ui.label(
                            RichText::new(relative_time(post.created_at))
                                .color(Color32::GRAY)
                                .size(11.),
                        );
                    });
                    ui.label(&post.content);
                    if ui.small_button("Open post").clicked() {
                        open_post = Some(post.clone());
                    }
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
        if let Some(author) = open_author {
            self.selected_author = author;
            self.tab = Tab::Profile;
        }
        if let Some(post) = open_post {
            self.selected_post = Some(post);
            self.tab = Tab::PostDetail;
        }
    }
}

fn relative_time(timestamp: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(timestamp);
    if timestamp > now.saturating_add(60) {
        return "future timestamp".into();
    }
    let elapsed = now.saturating_sub(timestamp);
    if elapsed < 60 {
        "now".into()
    } else if elapsed < 3600 {
        format!("{}m", elapsed / 60)
    } else if elapsed < 86400 {
        format!("{}h", elapsed / 3600)
    } else {
        format!("{}d", elapsed / 86400)
    }
}
