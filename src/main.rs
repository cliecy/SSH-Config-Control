use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use eframe::{
    App, Frame, NativeOptions, Renderer,
    egui::{
        self, Align, Color32, Context as EguiContext, CornerRadius, Frame as EguiFrame, Layout,
        Margin, RichText, ScrollArea, Stroke, TextEdit,
    },
};
use rfd::FileDialog;
use serde::{Deserialize, Serialize};

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        renderer: Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([980.0, 680.0])
            .with_title("SSH Host Manager"),
        ..Default::default()
    };

    eframe::run_native(
        "SSH Host Manager",
        options,
        Box::new(|cc| {
            configure_theme(&cc.egui_ctx);
            Ok(Box::new(SshManagerApp::new()))
        }),
    )
}

fn configure_theme(ctx: &EguiContext) {
    let mut visuals = egui::Visuals::light();
    visuals.window_fill = Color32::from_rgb(232, 223, 210);
    visuals.panel_fill = Color32::from_rgb(242, 236, 227);
    visuals.faint_bg_color = Color32::from_rgb(224, 213, 197);
    visuals.extreme_bg_color = Color32::from_rgb(251, 249, 245);
    visuals.hyperlink_color = Color32::from_rgb(13, 93, 92);
    visuals.selection.bg_fill = Color32::from_rgb(22, 111, 108);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(240, 247, 246));
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(241, 234, 224);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(73, 64, 55));
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(229, 220, 206);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(221, 209, 192);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(58, 52, 44));
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(197, 168, 122);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(41, 36, 31));
    visuals.widgets.active.bg_fill = Color32::from_rgb(160, 94, 54);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::from_rgb(252, 249, 244));
    visuals.widgets.open.bg_fill = Color32::from_rgb(210, 194, 172);
    visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(177, 160, 137));
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(12.0, 12.0);
    style.spacing.button_padding = egui::vec2(12.0, 9.0);
    style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(10);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(10);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(10);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(10);
    style.visuals.widgets.open.corner_radius = CornerRadius::same(10);
    ctx.set_style(style);
}

struct SshManagerApp {
    store_path: PathBuf,
    export_path: PathBuf,
    store: HostStore,
    store_writable: bool,
    selected_host: Option<String>,
    form: HostForm,
    filter_text: String,
    preview_text: String,
    diagnostics: Vec<String>,
    status: String,
}

impl SshManagerApp {
    fn new() -> Self {
        let store_path = default_store_path();
        let export_path = default_main_ssh_config_path();

        let (store, store_writable, status) = match HostStore::load_or_default(&store_path) {
            Ok(store) => {
                let status = if store.hosts.is_empty() {
                    format!("No saved hosts yet. Store: {}", store_path.display())
                } else {
                    format!(
                        "Loaded {} host(s) from {}",
                        store.hosts.len(),
                        store_path.display()
                    )
                };
                (store, true, status)
            }
            Err(err) => (
                HostStore::default(),
                false,
                format!(
                    "Failed to load store: {err:#}. Writes are disabled until the file is fixed or replaced."
                ),
            ),
        };

        let mut app = Self {
            store_path,
            export_path,
            store,
            store_writable,
            selected_host: None,
            form: HostForm::default(),
            filter_text: String::new(),
            preview_text: String::new(),
            diagnostics: Vec::new(),
            status,
        };
        app.refresh_preview();
        app
    }

    fn reset_form(&mut self) {
        self.selected_host = None;
        self.form = HostForm::default();
        self.refresh_preview();
    }

    fn refresh_preview(&mut self) {
        self.preview_text = match self.form.to_entry() {
            Ok((alias, host)) => render_host_block(&alias, &host),
            Err(_) => self.store.render_all(),
        };
        self.refresh_diagnostics();
    }

    fn refresh_diagnostics(&mut self) {
        self.diagnostics.clear();

        if !self.store_writable {
            self.diagnostics.push(format!(
                "Store is read-only because {} could not be parsed. Fix or replace the file before saving.",
                self.store_path.display()
            ));
        }

        if let Ok((alias, host)) = self.form.to_entry() {
            let expanded_key = expand_tilde_path(&host.identity_file);
            if expanded_key.exists() {
                self.diagnostics
                    .push(format!("Key file exists: {}", expanded_key.display()));
            } else {
                self.diagnostics.push(format!(
                    "Key file does not exist yet: {}",
                    expanded_key.display()
                ));
            }

            self.diagnostics.push(format!("Alias ready: {alias}"));
            if host.port.is_none() {
                self.diagnostics
                    .push("Port not set. SSH will use its default port.".to_owned());
            }
        } else if self.store.hosts.is_empty() {
            self.diagnostics
                .push("No saved hosts yet. Create one from the form on the left.".to_owned());
        } else {
            self.diagnostics
                .push("Select a host or fill the form to preview one entry.".to_owned());
        }

        let export_parent_missing = self
            .export_path
            .parent()
            .map(|parent| !parent.exists())
            .unwrap_or(false);
        if export_parent_missing {
            self.diagnostics.push(format!(
                "SSH config directory will be created if needed: {}",
                self.export_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .display()
            ));
        }
        self.diagnostics.push(format!(
            "Managed hosts will be written directly into {} with an automatic backup first.",
            self.export_path.display()
        ));
    }

    fn select_host(&mut self, alias: &str) {
        if let Some(host) = self.store.hosts.get(alias) {
            self.selected_host = Some(alias.to_owned());
            self.form = HostForm::from_stored(alias, host);
            self.refresh_preview();
        }
    }

    fn save_current_host(&mut self) {
        if !self.store_writable {
            self.status = format!(
                "Cannot save because the store at {} failed to load. Fix the file first.",
                self.store_path.display()
            );
            return;
        }

        match self.form.to_entry() {
            Ok((alias, host)) => {
                let message = if self.store.hosts.contains_key(&alias) {
                    format!("Updated host '{alias}'")
                } else {
                    format!("Created host '{alias}'")
                };

                let mut next_store = self.store.clone();
                next_store.hosts.insert(alias.clone(), host);

                match next_store.save(&self.store_path) {
                    Ok(()) => {
                        self.store = next_store;
                        self.selected_host = Some(alias);
                        self.status = message;
                        self.refresh_preview();
                    }
                    Err(err) => self.status = format!("Save failed: {err:#}"),
                }
            }
            Err(err) => self.status = err.to_string(),
        }
    }

    fn delete_current_host(&mut self) {
        if !self.store_writable {
            self.status = format!(
                "Cannot delete because the store at {} failed to load. Fix the file first.",
                self.store_path.display()
            );
            return;
        }

        let Some(alias) = self.selected_host.clone() else {
            self.status = "Select a host before deleting.".to_owned();
            return;
        };

        let mut next_store = self.store.clone();
        if next_store.hosts.remove(&alias).is_none() {
            self.status = format!("Host '{alias}' was not found.");
            return;
        }

        match next_store.save(&self.store_path) {
            Ok(()) => {
                self.store = next_store;
                self.reset_form();
                self.status = format!("Deleted host '{alias}'");
            }
            Err(err) => self.status = format!("Delete failed: {err:#}"),
        }
    }

    fn reload_store(&mut self) {
        match HostStore::load_or_default(&self.store_path) {
            Ok(store) => {
                self.store = store;
                self.store_writable = true;
                if let Some(alias) = self.selected_host.clone() {
                    if self.store.hosts.contains_key(&alias) {
                        self.select_host(&alias);
                    } else {
                        self.reset_form();
                    }
                } else {
                    self.refresh_preview();
                }
                self.status = format!("Reloaded store from {}", self.store_path.display());
            }
            Err(err) => {
                self.store_writable = false;
                self.status = format!(
                    "Reload failed: {err:#}. Writes are disabled until the store is fixed."
                );
                self.refresh_diagnostics();
            }
        }
    }

    fn import_main_ssh_config(&mut self) {
        if !self.store_writable {
            self.status = format!(
                "Cannot import into a broken store at {}. Fix the file first.",
                self.store_path.display()
            );
            return;
        }

        let main_config = default_main_ssh_config_path();
        match import_ssh_config_file(&main_config) {
            Ok(imported) => {
                if imported.is_empty() {
                    self.status = format!(
                        "No simple host blocks were found in {}",
                        main_config.display()
                    );
                    return;
                }

                let mut next_store = self.store.clone();
                let (inserted, skipped) = merge_imported_hosts(&mut next_store, imported);

                match next_store.save(&self.store_path) {
                    Ok(()) => {
                        self.store = next_store;
                        self.status = format!(
                            "Imported {} new host(s), skipped {} existing host(s) from {}",
                            inserted,
                            skipped,
                            main_config.display()
                        );
                        self.refresh_preview();
                    }
                    Err(err) => self.status = format!("Import save failed: {err:#}"),
                }
            }
            Err(err) => {
                self.status = format!("Import failed: {err:#}");
            }
        }
    }

    fn export_rendered_config(&mut self) {
        let rendered = self.store.render_all();
        if rendered.is_empty() {
            self.status = "Nothing to apply. Add at least one host first.".to_owned();
            return;
        }

        let main_config = self.export_path.clone();
        match sync_managed_hosts_into_main_config(&main_config, &rendered) {
            Ok(backup_path) => {
                self.status = match backup_path {
                    Some(path) => format!(
                        "Applied managed hosts directly to {}. Backup created at {}",
                        main_config.display(),
                        path.display()
                    ),
                    None => format!(
                        "Applied managed hosts directly to {}",
                        main_config.display()
                    ),
                };
                self.refresh_diagnostics();
            }
            Err(err) => {
                self.status = format!("Apply failed: {err:#}");
            }
        }
    }

    fn pick_identity_file(&mut self) {
        let dialog = if self.form.identity_file.trim().is_empty() {
            FileDialog::new()
        } else {
            let start = expand_tilde_path(self.form.identity_file.trim());
            if start.exists() {
                FileDialog::new().set_directory(start.parent().unwrap_or(&start))
            } else {
                FileDialog::new()
            }
        };

        if let Some(path) = dialog.pick_file() {
            self.form.identity_file = path.display().to_string();
            self.status = format!("Selected key file {}", path.display());
            self.refresh_preview();
        }
    }

    fn duplicate_selected_host(&mut self) {
        let Some(alias) = self.selected_host.clone() else {
            self.status = "Select a host before duplicating.".to_owned();
            return;
        };

        if let Some(host) = self.store.hosts.get(&alias) {
            let new_alias = unique_alias(&self.store.hosts, &format!("{alias}-copy"));
            self.form = HostForm::from_stored(&new_alias, host);
            self.selected_host = None;
            self.status = format!("Duplicated '{alias}' into form as '{new_alias}'");
            self.refresh_preview();
        }
    }
}

impl App for SshManagerApp {
    fn update(&mut self, ctx: &EguiContext, _frame: &mut Frame) {
        egui::TopBottomPanel::top("top_bar")
            .resizable(false)
            .show(ctx, |ui| {
                EguiFrame::new()
                    .fill(Color32::from_rgb(95, 61, 43))
                    .corner_radius(CornerRadius::same(18))
                    .inner_margin(Margin::symmetric(18, 16))
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.heading(
                                RichText::new("SSH Host Manager")
                                    .size(30.0)
                                    .color(Color32::from_rgb(250, 245, 238)),
                            );
                            ui.label(
                                RichText::new(
                                    "Manage servers, keys, and write the managed host block straight into your main SSH config.",
                                )
                                .color(Color32::from_rgb(239, 226, 210)),
                            );
                        });
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            pill_label(
                                ui,
                                "Store",
                                &self.store_path.display().to_string(),
                                Color32::from_rgb(176, 128, 86),
                                Color32::from_rgb(251, 247, 241),
                            );
                            pill_label(
                                ui,
                                "Main Config",
                                &self.export_path.display().to_string(),
                                Color32::from_rgb(28, 108, 104),
                                Color32::from_rgb(243, 248, 247),
                            );
                            if !self.store_writable {
                                pill_label(
                                    ui,
                                    "State",
                                    "Read-only until load errors are fixed",
                                    Color32::from_rgb(155, 54, 40),
                                    Color32::from_rgb(252, 241, 239),
                                );
                            }
                        });
                    });
            });

        egui::TopBottomPanel::bottom("status_bar")
            .resizable(false)
            .show(ctx, |ui| {
                EguiFrame::new()
                    .fill(status_background(&self.status))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(&self.status)
                                .strong()
                                .color(Color32::from_rgb(48, 40, 32)),
                        );
                    });
            });

        egui::SidePanel::left("host_list")
            .resizable(true)
            .default_width(320.0)
            .show(ctx, |ui| {
                section_frame(Color32::from_rgb(226, 213, 195)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading(RichText::new("Servers").size(22.0));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Import").clicked() {
                                self.import_main_ssh_config();
                            }
                            if ui.button("Reload").clicked() {
                                self.reload_store();
                            }
                            if ui.button("Add Host").clicked() {
                                self.reset_form();
                                self.status = "Creating a new host entry. Fill the form and click Create Host.".to_owned();
                            }
                        });
                    });

                    ui.add(
                        TextEdit::singleline(&mut self.filter_text)
                            .hint_text("Filter by alias, hostname, or user"),
                    );

                    ui.label(
                        RichText::new(format!("{} hosts", self.store.hosts.len()))
                            .color(Color32::from_rgb(95, 81, 65)),
                    );

                    ui.add_space(4.0);

                    ScrollArea::vertical().show(ui, |ui| {
                        let filter = self.filter_text.to_lowercase();
                        let entries: Vec<(String, String, String, bool)> = self
                            .store
                            .hosts
                            .iter()
                            .filter_map(|(alias, host)| {
                                let user = host.user.clone().unwrap_or_else(|| "-".to_owned());
                                let haystack =
                                    format!("{alias} {} {user}", host.hostname).to_lowercase();
                                if filter.is_empty() || haystack.contains(&filter) {
                                    Some((
                                        alias.clone(),
                                        host.hostname.clone(),
                                        user,
                                        expand_tilde_path(&host.identity_file).exists(),
                                    ))
                                } else {
                                    None
                                }
                            })
                            .collect();

                        if entries.is_empty() {
                            muted_box(ui, "No matching servers.");
                        } else {
                            for (alias, hostname, user, key_exists) in entries {
                                let selected =
                                    self.selected_host.as_deref() == Some(alias.as_str());
                                let response = server_list_item(
                                    ui, &alias, &hostname, &user, key_exists, selected,
                                );
                                if response.clicked() {
                                    self.select_host(&alias);
                                }
                                ui.add_space(6.0);
                            }
                        }
                    });
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |columns| {
                let left = &mut columns[0];
                section_frame(Color32::from_rgb(241, 233, 220)).show(left, |ui| {
                    let creating_new = self.selected_host.is_none();
                    let panel_title = if creating_new { "Create Host" } else { "Edit Host" };
                    let save_label = if creating_new { "Create Host" } else { "Update Host" };

                    ui.heading(RichText::new(panel_title).size(22.0));
                    ui.label(
                        RichText::new(if creating_new {
                            "Fill in the fields below, then click Create Host."
                        } else {
                            "Update the selected host, then click Update Host."
                        })
                            .color(Color32::from_rgb(105, 89, 72)),
                    );

                    ui.separator();

                    labeled_field(ui, "Alias");
                    if ui
                        .add(
                            TextEdit::singleline(&mut self.form.alias)
                                .hint_text("example: prod-web"),
                        )
                        .changed()
                    {
                        self.refresh_preview();
                    }

                    labeled_field(ui, "HostName / IP");
                    if ui
                        .add(
                            TextEdit::singleline(&mut self.form.hostname)
                                .hint_text("example: 192.168.1.10"),
                        )
                        .changed()
                    {
                        self.refresh_preview();
                    }

                    labeled_field(ui, "User");
                    if ui
                        .add(TextEdit::singleline(&mut self.form.user).hint_text("example: ubuntu"))
                        .changed()
                    {
                        self.refresh_preview();
                    }

                    labeled_field(ui, "Port");
                    if ui
                        .add(TextEdit::singleline(&mut self.form.port).hint_text("default: 22"))
                        .changed()
                    {
                        self.refresh_preview();
                    }

                    labeled_field(ui, "IdentityFile");
                    ui.horizontal(|ui| {
                        let changed = ui
                            .add(
                                TextEdit::singleline(&mut self.form.identity_file)
                                    .hint_text(identity_file_hint())
                                    .desired_width(360.0),
                            )
                            .changed();
                        if changed {
                            self.refresh_preview();
                        }
                        if ui.button("Choose File").clicked() {
                            self.pick_identity_file();
                        }
                    });

                    if ui
                        .checkbox(&mut self.form.identities_only, "Force this key only")
                        .changed()
                    {
                        self.refresh_preview();
                    }

                    labeled_field(ui, "Note");
                    if ui
                        .add(
                            TextEdit::multiline(&mut self.form.note)
                                .desired_rows(3)
                                .hint_text("optional memo for this server"),
                        )
                        .changed()
                    {
                        self.refresh_preview();
                    }

                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(save_label).clicked() {
                            self.save_current_host();
                        }
                        if ui.button("Duplicate").clicked() {
                            self.duplicate_selected_host();
                        }
                        if ui.button("Delete").clicked() {
                            self.delete_current_host();
                        }
                        if ui.button("Clear").clicked() {
                            self.reset_form();
                            self.status = "Form cleared.".to_owned();
                        }
                    });
                });

                left.add_space(10.0);
                section_frame(Color32::from_rgb(221, 232, 228)).show(left, |ui| {
                    ui.heading(RichText::new("Validation").size(20.0));
                    ui.label(
                        RichText::new("Quick checks for the current form and main config path.")
                            .color(Color32::from_rgb(77, 89, 86)),
                    );
                    ui.separator();
                    for line in &self.diagnostics {
                        ui.label(line);
                    }
                });

                let right = &mut columns[1];
                section_frame(Color32::from_rgb(44, 50, 56)).show(right, |ui| {
                    ui.heading(
                        RichText::new("Generated SSH Config")
                            .size(22.0)
                            .color(Color32::from_rgb(247, 243, 237)),
                    );
                    ui.label(
                        RichText::new(format!(
                            "Live preview of the managed block that will be written into {}.",
                            self.export_path.display()
                        ))
                            .color(Color32::from_rgb(186, 196, 206)),
                    );
                    ui.separator();

                    ScrollArea::vertical().show(ui, |ui| {
                        ui.add(
                            TextEdit::multiline(&mut self.preview_text)
                                .desired_rows(22)
                                .interactive(false)
                                .font(egui::TextStyle::Monospace),
                        );
                    });
                });

                right.add_space(10.0);
                section_frame(Color32::from_rgb(233, 227, 212)).show(right, |ui| {
                    ui.heading(RichText::new("Apply To Main Config").size(20.0));
                    ui.label(
                        RichText::new(format!(
                            "This writes a managed block directly into {} and makes a backup first.",
                            self.export_path.display()
                        ))
                        .color(Color32::from_rgb(92, 80, 66)),
                    );
                    ui.separator();

                    ui.label(RichText::new("Main SSH Config").strong());
                    let mut export_path_text = self.export_path.display().to_string();
                    ui.add(
                        TextEdit::singleline(&mut export_path_text)
                            .interactive(false)
                            .desired_width(f32::INFINITY),
                    );

                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Apply Managed Hosts Now").clicked() {
                            self.export_rendered_config();
                        }
                        if ui.button("Reset To Default Config Path").clicked() {
                            self.export_path = default_main_ssh_config_path();
                            self.status =
                                format!("Main config path reset to {}", self.export_path.display());
                            self.refresh_diagnostics();
                        }
                    });
                });

                right.add_space(10.0);
                section_frame(Color32::from_rgb(210, 220, 230)).show(right, |ui| {
                    ui.heading(RichText::new("Workflow").size(20.0));
                    ui.separator();
                    ui.label("1. Create hosts or import simple Host blocks.");
                    ui.label("2. Save hosts to the local registry.");
                    ui.label(format!(
                        "3. Apply the managed host block directly to {}.",
                        self.export_path.display()
                    ));
                    ui.label("4. A backup is created before every config write.");
                    ui.label("5. Connect with: ssh <alias>");
                });
            });
        });
    }
}

fn section_frame(fill: Color32) -> EguiFrame {
    EguiFrame::new()
        .fill(fill)
        .stroke(Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(90, 75, 58, 40),
        ))
        .corner_radius(CornerRadius::same(18))
        .inner_margin(Margin::same(16))
}

fn muted_box(ui: &mut egui::Ui, text: &str) {
    EguiFrame::new()
        .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 120))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(Color32::from_rgb(98, 86, 72)));
        });
}

fn pill_label(ui: &mut egui::Ui, label: &str, value: &str, fill: Color32, text_color: Color32) {
    EguiFrame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(label).strong().color(text_color));
                ui.label(RichText::new(value).color(text_color));
            });
        });
}

fn labeled_field(ui: &mut egui::Ui, label: &str) {
    ui.label(
        RichText::new(label)
            .strong()
            .color(Color32::from_rgb(70, 58, 45)),
    );
}

fn server_list_item(
    ui: &mut egui::Ui,
    alias: &str,
    hostname: &str,
    user: &str,
    key_exists: bool,
    selected: bool,
) -> egui::Response {
    let fill = if selected {
        Color32::from_rgb(28, 108, 104)
    } else {
        Color32::from_rgb(247, 242, 235)
    };
    let stroke = if selected {
        Stroke::new(1.5, Color32::from_rgb(17, 75, 73))
    } else {
        Stroke::new(1.0, Color32::from_rgb(212, 199, 181))
    };
    let title_color = if selected {
        Color32::from_rgb(245, 250, 249)
    } else {
        Color32::from_rgb(53, 46, 40)
    };
    let meta_color = if selected {
        Color32::from_rgb(216, 236, 233)
    } else {
        Color32::from_rgb(110, 97, 82)
    };
    let key_text = if key_exists {
        "key ready"
    } else {
        "key missing"
    };

    let frame = EguiFrame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(12));

    frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.label(RichText::new(alias).strong().size(18.0).color(title_color));
                ui.label(RichText::new(hostname).color(meta_color));
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("user: {user}")).color(meta_color));
                    ui.label(RichText::new(key_text).color(meta_color));
                });
            });
        })
        .response
        .interact(egui::Sense::click())
}

fn status_background(status: &str) -> Color32 {
    let lower = status.to_lowercase();
    if lower.contains("failed")
        || lower.contains("cannot")
        || lower.contains("error")
        || lower.contains("read-only")
    {
        Color32::from_rgb(237, 199, 191)
    } else if lower.contains("created")
        || lower.contains("updated")
        || lower.contains("saved")
        || lower.contains("imported")
        || lower.contains("exported")
        || lower.contains("added")
    {
        Color32::from_rgb(197, 225, 216)
    } else {
        Color32::from_rgb(227, 214, 191)
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct HostStore {
    #[serde(default)]
    hosts: BTreeMap<String, StoredHost>,
}

impl HostStore {
    fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read store {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    }

    fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::default())
        }
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create store directory {}", parent.display())
            })?;
        }
        let output = toml::to_string_pretty(self).context("failed to serialize store")?;
        write_text_file_atomically(path, &output)
    }

    fn render_all(&self) -> String {
        let mut out = String::new();
        for (name, host) in &self.hosts {
            out.push_str(&render_host_block(name, host));
        }
        out
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
struct StoredHost {
    hostname: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    identity_file: String,
    #[serde(default = "default_true")]
    identities_only: bool,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Clone)]
struct HostForm {
    alias: String,
    hostname: String,
    user: String,
    port: String,
    identity_file: String,
    identities_only: bool,
    note: String,
}

impl Default for HostForm {
    fn default() -> Self {
        Self {
            alias: String::new(),
            hostname: String::new(),
            user: String::new(),
            port: String::new(),
            identity_file: String::new(),
            identities_only: true,
            note: String::new(),
        }
    }
}

impl HostForm {
    fn from_stored(alias: &str, host: &StoredHost) -> Self {
        Self {
            alias: alias.to_owned(),
            hostname: host.hostname.clone(),
            user: host.user.clone().unwrap_or_default(),
            port: host.port.map(|v| v.to_string()).unwrap_or_default(),
            identity_file: host.identity_file.clone(),
            identities_only: host.identities_only,
            note: host.note.clone().unwrap_or_default(),
        }
    }

    fn to_entry(&self) -> Result<(String, StoredHost)> {
        let alias = self.alias.trim();
        if alias.is_empty() {
            bail!("Alias is required.");
        }

        let hostname = self.hostname.trim();
        if hostname.is_empty() {
            bail!("HostName is required.");
        }

        let identity_file = self.identity_file.trim();
        if identity_file.is_empty() {
            bail!("IdentityFile is required.");
        }

        let port = if self.port.trim().is_empty() {
            None
        } else {
            Some(
                self.port
                    .trim()
                    .parse::<u16>()
                    .context("Port must be a number between 0 and 65535.")?,
            )
        };

        Ok((
            alias.to_owned(),
            StoredHost {
                hostname: hostname.to_owned(),
                user: optional_string(&self.user),
                port,
                identity_file: identity_file.to_owned(),
                identities_only: self.identities_only,
                note: optional_string(&self.note),
            },
        ))
    }
}

fn render_host_block(name: &str, host: &StoredHost) -> String {
    let mut out = String::new();
    if let Some(note) = &host.note {
        out.push_str("# ");
        out.push_str(note);
        out.push('\n');
    }
    out.push_str("Host ");
    out.push_str(name);
    out.push('\n');
    out.push_str("  HostName ");
    out.push_str(&host.hostname);
    out.push('\n');
    if let Some(user) = &host.user {
        out.push_str("  User ");
        out.push_str(user);
        out.push('\n');
    }
    if let Some(port) = host.port {
        out.push_str("  Port ");
        out.push_str(&port.to_string());
        out.push('\n');
    }
    out.push_str("  IdentityFile ");
    out.push_str(&ssh_config_quote(&host.identity_file));
    out.push('\n');
    if host.identities_only {
        out.push_str("  IdentitiesOnly yes\n");
    }
    out.push('\n');
    out
}

fn import_ssh_config_file(path: &Path) -> Result<Vec<(String, StoredHost)>> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read SSH config {}", path.display()))?;
    Ok(parse_ssh_config(&raw))
}

fn parse_ssh_config(raw: &str) -> Vec<(String, StoredHost)> {
    let mut imported = Vec::new();
    let mut current_alias: Option<String> = None;
    let mut current_host = StoredHost {
        hostname: String::new(),
        user: None,
        port: None,
        identity_file: String::new(),
        identities_only: true,
        note: None,
    };

    for line in raw.lines() {
        let trimmed_storage = strip_inline_comment(line);
        let trimmed = trimmed_storage.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let Some(keyword) = parts.next() else {
            continue;
        };
        let rest = unquote_ssh_value(&parts.collect::<Vec<_>>().join(" "));
        if rest.is_empty() {
            continue;
        }

        if keyword.eq_ignore_ascii_case("Host") {
            if let Some(alias) = current_alias.take()
                && is_importable_host(&alias, &current_host)
            {
                imported.push((alias, current_host.clone()));
            }

            current_host = StoredHost {
                hostname: String::new(),
                user: None,
                port: None,
                identity_file: String::new(),
                identities_only: true,
                note: None,
            };

            let aliases: Vec<&str> = rest.split_whitespace().collect();
            current_alias = aliases
                .iter()
                .find(|alias| !contains_pattern(alias))
                .map(|alias| (*alias).to_owned());
            continue;
        }

        if current_alias.is_none() {
            continue;
        }

        if keyword.eq_ignore_ascii_case("HostName") {
            current_host.hostname = rest;
        } else if keyword.eq_ignore_ascii_case("User") {
            current_host.user = Some(rest);
        } else if keyword.eq_ignore_ascii_case("Port") {
            current_host.port = rest.parse::<u16>().ok();
        } else if keyword.eq_ignore_ascii_case("IdentityFile") {
            current_host.identity_file = rest;
        } else if keyword.eq_ignore_ascii_case("IdentitiesOnly") {
            current_host.identities_only = rest.eq_ignore_ascii_case("yes");
        }
    }

    if let Some(alias) = current_alias
        && is_importable_host(&alias, &current_host)
    {
        imported.push((alias, current_host));
    }

    imported
}

fn is_importable_host(alias: &str, host: &StoredHost) -> bool {
    !alias.is_empty()
        && !contains_pattern(alias)
        && !host.hostname.is_empty()
        && !host.identity_file.is_empty()
}

fn contains_pattern(alias: &str) -> bool {
    alias.contains('*') || alias.contains('?') || alias.contains('!')
}

fn backup_file_if_exists(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_secs();

    let backup_name = format!(
        "{}.bak-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        timestamp
    );
    let backup_path = path.with_file_name(backup_name);
    fs::copy(path, &backup_path).with_context(|| {
        format!(
            "failed to create backup from {} to {}",
            path.display(),
            backup_path.display()
        )
    })?;
    Ok(Some(backup_path))
}

fn unique_alias(hosts: &BTreeMap<String, StoredHost>, base: &str) -> String {
    if !hosts.contains_key(base) {
        return base.to_owned();
    }

    let mut index = 2usize;
    loop {
        let candidate = format!("{base}-{index}");
        if !hosts.contains_key(&candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn merge_imported_hosts(
    store: &mut HostStore,
    imported: Vec<(String, StoredHost)>,
) -> (usize, usize) {
    let mut inserted = 0usize;
    let mut skipped = 0usize;

    for (alias, host) in imported {
        if let std::collections::btree_map::Entry::Vacant(entry) = store.hosts.entry(alias) {
            entry.insert(host);
            inserted += 1;
        } else {
            skipped += 1;
        }
    }

    (inserted, skipped)
}

const MANAGED_BLOCK_START: &str = "# >>> rurs-managed hosts >>>";
const MANAGED_BLOCK_END: &str = "# <<< rurs-managed hosts <<<";

fn sync_managed_hosts_into_main_config(
    main_config_path: &Path,
    managed_hosts: &str,
) -> Result<Option<PathBuf>> {
    if let Some(parent) = main_config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    let existing = match fs::read_to_string(main_config_path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "failed to read main SSH config {}",
                    main_config_path.display()
                )
            });
        }
    };

    let backup_path = backup_file_if_exists(main_config_path)?;
    let updated = replace_or_append_managed_block(&existing, managed_hosts);
    write_text_file_atomically(main_config_path, &updated)?;
    Ok(backup_path)
}

fn replace_or_append_managed_block(existing: &str, managed_hosts: &str) -> String {
    let mut block = String::new();
    block.push_str(MANAGED_BLOCK_START);
    block.push('\n');
    block.push_str(managed_hosts.trim_end());
    block.push('\n');
    block.push_str(MANAGED_BLOCK_END);
    block.push('\n');

    if let Some(start) = existing.find(MANAGED_BLOCK_START)
        && let Some(end_offset) = existing[start..].find(MANAGED_BLOCK_END)
    {
        let end = start + end_offset + MANAGED_BLOCK_END.len();
        let mut updated = String::new();
        updated.push_str(existing[..start].trim_end());
        if !updated.is_empty() {
            updated.push_str("\n\n");
        }
        updated.push_str(&block);
        let tail = existing[end..].trim_start_matches('\n');
        if !tail.trim().is_empty() {
            updated.push('\n');
            updated.push_str(tail);
        }
        return updated;
    }

    let mut updated = existing.trim_end().to_owned();
    if !updated.is_empty() {
        updated.push_str("\n\n");
    }
    updated.push_str(&block);
    updated
}

fn default_store_path() -> PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        return config_dir.join("rurs_test").join("hosts.toml");
    }
    PathBuf::from("hosts.toml")
}

fn default_main_ssh_config_path() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        return home.join(".ssh").join("config");
    }
    PathBuf::from(".ssh/config")
}

fn identity_file_hint() -> &'static str {
    if cfg!(windows) {
        r"example: C:\Users\you\.ssh\id_ed25519"
    } else {
        "example: ~/.ssh/id_ed25519_prod"
    }
}

fn default_true() -> bool {
    true
}

fn ssh_config_quote(value: &str) -> String {
    if value.contains(char::is_whitespace) || value.contains('"') {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

fn optional_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn expand_tilde_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }

    if let Some(stripped) = raw.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(stripped);
    }

    if let Some(stripped) = raw.strip_prefix("$HOME/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(stripped);
    }

    if let Some(home) = dirs::home_dir() {
        if let Some(stripped) = raw.strip_prefix(r"%USERPROFILE%\") {
            return home.join(stripped);
        }
        if let Some(stripped) = raw.strip_prefix("%USERPROFILE%/") {
            return home.join(stripped);
        }
    }

    PathBuf::from(raw)
}

fn strip_inline_comment(line: &str) -> String {
    let mut out = String::new();
    let mut in_quotes = false;
    let mut escaped = false;

    for ch in line.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' => {
                escaped = true;
                out.push(ch);
            }
            '"' => {
                in_quotes = !in_quotes;
                out.push(ch);
            }
            '#' if !in_quotes => break,
            _ => out.push(ch),
        }
    }

    out
}

fn unquote_ssh_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].replace("\\\"", "\"")
    } else {
        trimmed.to_owned()
    }
}

fn write_text_file_atomically(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create directory {}", parent.display()))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_nanos();
    let temp_name = format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        timestamp
    );
    let temp_path = parent.join(temp_name);

    fs::write(&temp_path, content)
        .with_context(|| format!("failed to write temporary file {}", temp_path.display()))?;
    if let Err(err) = fs::rename(&temp_path, path) {
        #[cfg(windows)]
        {
            if path.exists() {
                fs::remove_file(path).with_context(|| {
                    format!("failed to replace existing file {}", path.display())
                })?;
                fs::rename(&temp_path, path).with_context(|| {
                    format!(
                        "failed to move temporary file {} into {}",
                        temp_path.display(),
                        path.display()
                    )
                })?;
                return Ok(());
            }
        }

        let _ = fs::remove_file(&temp_path);
        return Err(err).with_context(|| {
            format!(
                "failed to move temporary file {} into {}",
                temp_path.display(),
                path.display()
            )
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_hosts_from_ssh_config() {
        let raw = r#"
            Host prod-web
              HostName 203.0.113.10
              User ubuntu
              Port 22
              IdentityFile ~/.ssh/id_ed25519_prod
              IdentitiesOnly yes

            Host *
              ForwardAgent yes

            Host git-internal
              HostName git.example.com
              User git
              IdentityFile ~/.ssh/id_ed25519_git
        "#;

        let parsed = parse_ssh_config(raw);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "prod-web");
        assert_eq!(parsed[0].1.hostname, "203.0.113.10");
        assert_eq!(parsed[1].0, "git-internal");
        assert_eq!(parsed[1].1.user.as_deref(), Some("git"));
    }

    #[test]
    fn skips_hosts_without_key_or_hostname() {
        let raw = r#"
            Host incomplete
              HostName 203.0.113.10

            Host missing-hostname
              IdentityFile ~/.ssh/id_missing
        "#;

        let parsed = parse_ssh_config(raw);
        assert!(parsed.is_empty());
    }

    #[test]
    fn renders_host_block() {
        let host = StoredHost {
            hostname: "203.0.113.10".to_owned(),
            user: Some("ubuntu".to_owned()),
            port: Some(22),
            identity_file: "~/.ssh/id_ed25519_prod".to_owned(),
            identities_only: true,
            note: Some("prod".to_owned()),
        };

        let rendered = render_host_block("prod-web", &host);
        assert!(rendered.contains("Host prod-web"));
        assert!(rendered.contains("IdentityFile ~/.ssh/id_ed25519_prod"));
    }

    #[test]
    fn render_quotes_paths_with_spaces() {
        let host = StoredHost {
            hostname: "example.com".to_owned(),
            user: None,
            port: None,
            identity_file: "/Users/test/My Keys/id key".to_owned(),
            identities_only: true,
            note: None,
        };

        let rendered = render_host_block("demo", &host);
        assert!(rendered.contains("IdentityFile \"/Users/test/My Keys/id key\""));
    }

    #[test]
    fn parse_strips_inline_comments_and_quotes() {
        let raw = r#"
            Host quoted
              HostName "host.example.com" # comment
              IdentityFile "/Users/test/My Keys/id key" # another comment
        "#;

        let parsed = parse_ssh_config(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].1.hostname, "host.example.com");
        assert_eq!(parsed[0].1.identity_file, "/Users/test/My Keys/id key");
    }

    #[test]
    fn merge_import_skips_existing_hosts() {
        let existing = StoredHost {
            hostname: "old.example.com".to_owned(),
            user: Some("ubuntu".to_owned()),
            port: None,
            identity_file: "~/.ssh/old".to_owned(),
            identities_only: true,
            note: Some("keep me".to_owned()),
        };
        let imported = StoredHost {
            hostname: "new.example.com".to_owned(),
            user: Some("root".to_owned()),
            port: Some(22),
            identity_file: "~/.ssh/new".to_owned(),
            identities_only: true,
            note: None,
        };

        let mut store = HostStore::default();
        store.hosts.insert("prod".to_owned(), existing.clone());
        let (inserted, skipped) =
            merge_imported_hosts(&mut store, vec![("prod".to_owned(), imported)]);

        assert_eq!(inserted, 0);
        assert_eq!(skipped, 1);
        assert_eq!(store.hosts.get("prod"), Some(&existing));
    }

    #[test]
    fn atomic_write_replaces_file_contents() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rurs-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let file = temp_dir.join("hosts.toml");

        write_text_file_atomically(&file, "first").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "first");

        write_text_file_atomically(&file, "second").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "second");

        let _ = fs::remove_file(&file);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn replace_or_append_managed_block_appends_when_missing() {
        let existing = "Host manual\n  HostName example.com\n";
        let updated = replace_or_append_managed_block(existing, "Host managed\n  HostName managed");

        assert!(updated.contains("Host manual"));
        assert!(updated.contains(MANAGED_BLOCK_START));
        assert!(updated.contains("Host managed"));
        assert!(updated.contains(MANAGED_BLOCK_END));
    }

    #[test]
    fn replace_or_append_managed_block_replaces_existing_block() {
        let existing = format!(
            "Host manual\n  HostName example.com\n\n{MANAGED_BLOCK_START}\nHost old\n  HostName old\n{MANAGED_BLOCK_END}\n"
        );
        let updated = replace_or_append_managed_block(&existing, "Host new\n  HostName new");

        assert!(updated.contains("Host manual"));
        assert!(updated.contains("Host new"));
        assert!(!updated.contains("Host old"));
    }
}
