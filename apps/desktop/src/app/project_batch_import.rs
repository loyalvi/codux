use super::project_actions::FilePickerOpenRequest;
use super::project_batch_scan::{ProjectKind, ScanResult, ScannedProject, scan_git_projects};
use super::*;
use gpui_component::{
    checkbox::Checkbox,
    input::{Input, InputState},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
pub(in crate::app) struct ProjectBatchImport {
    root: Option<PathBuf>,
    candidates: Vec<ScannedProject>,
    existing: BTreeMap<PathBuf, bool>,
    selected: BTreeSet<PathBuf>,
    cancel: Option<Arc<AtomicBool>>,
    skipped: usize,
    importing: bool,
    error: Option<String>,
    summary: Option<(usize, usize)>,
}

impl Drop for ProjectBatchImport {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl CoduxApp {
    pub(super) fn project_batch_import_entry(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let locale = locale_from_language_setting(&self.state.settings.language);
        dialog_secondary_button(
            "project-batch-entry",
            translate(&locale, "project.batch.title", "Import Git projects…"),
            cx,
            |app, _, _, cx| {
                app.project_batch_import = Some(ProjectBatchImport::default());
                cx.notify();
            },
        )
    }

    fn choose_project_batch_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let last_root = self
            .project_batch_import
            .as_ref()
            .and_then(|batch| batch.root.clone())
            .or_else(|| {
                std::fs::read_to_string(self.state.support_dir.join("batch-import-last-root.txt"))
                    .ok()
                    .map(PathBuf::from)
            });
        self.open_file_picker_window(
            FilePickerOpenRequest {
                mode: FilePickerMode::OpenFolder,
                target: FilePickerTarget::ProjectBatchRoot,
                runtime_target: ProjectRuntimeTarget::Local,
                start_path: last_root.map(|path| path.to_string_lossy().into_owned()),
                default_filename: None,
            },
            window,
            cx,
        );
    }

    pub(super) fn reject_project_batch_remote_root(&mut self, cx: &mut Context<Self>) {
        let locale = locale_from_language_setting(&self.state.settings.language);
        if let Some(batch) = self.project_batch_import.as_mut() {
            batch.error = Some(translate(
                &locale,
                "project.batch.local_only",
                "Select a folder on this computer.",
            ));
        }
        cx.notify();
    }

    pub(super) fn start_project_batch_scan(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let Some(batch) = self.project_batch_import.as_mut() else {
            return;
        };
        if batch.importing {
            return;
        }
        if let Some(old) = batch.cancel.take() {
            old.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        batch.root = Some(root.clone());
        batch.cancel = Some(cancel.clone());
        batch.candidates.clear();
        batch.selected.clear();
        batch.existing.clear();
        batch.skipped = 0;
        batch.error = None;
        batch.summary = None;
        let support_dir = self.state.support_dir.clone();
        let known = self.state.projects.clone();
        let scan_cancel = cancel.clone();
        cx.notify();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::run_limited_blocking(move || {
                let result = scan_git_projects(&root, &scan_cancel)?;
                let existing = existing_candidates(&result, &known, &support_dir);
                if !scan_cancel.load(Ordering::Relaxed) {
                    let _ = std::fs::write(
                        support_dir.join("batch-import-last-root.txt"),
                        root.to_string_lossy().as_bytes(),
                    );
                }
                Ok::<_, String>((result, existing))
            })
            .await
            .unwrap_or_else(|error| Err(format!("Scan failed: {error}")));
            let _ = this.update(cx, |app, cx| {
                let Some(batch) = app.project_batch_import.as_mut() else {
                    return;
                };
                if batch
                    .cancel
                    .as_ref()
                    .is_none_or(|active| !Arc::ptr_eq(active, &cancel))
                {
                    return;
                }
                batch.cancel = None;
                match result {
                    Ok((result, existing)) => {
                        batch.candidates = result.projects;
                        batch.skipped = result.skipped;
                        batch.existing = existing;
                    }
                    Err(error) => batch.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn cancel_project_batch_scan(&mut self, cx: &mut Context<Self>) {
        let locale = locale_from_language_setting(&self.state.settings.language);
        if let Some(batch) = self.project_batch_import.as_mut() {
            if let Some(cancel) = batch.cancel.take() {
                cancel.store(true, Ordering::Relaxed);
            }
            batch.error = Some(translate(
                &locale,
                "project.batch.cancelled",
                "Scan cancelled.",
            ));
        }
        cx.notify();
    }

    fn accept_project_batch_import(&mut self, cx: &mut Context<Self>) {
        let Some(batch) = self.project_batch_import.as_mut() else {
            return;
        };
        if batch.importing || batch.cancel.is_some() {
            return;
        }
        let paths: Vec<_> = batch
            .candidates
            .iter()
            .filter(|project| {
                batch.selected.contains(&project.path)
                    && !batch.existing.contains_key(&project.path)
            })
            .map(|project| {
                (
                    project.name.clone(),
                    project.path.to_string_lossy().into_owned(),
                )
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        batch.importing = true;
        batch.error = None;
        batch.summary = None;
        let runtime = self.runtime_service.clone();
        let candidates = batch.candidates.clone();
        let support_dir = self.state.support_dir.clone();
        // Hold the weak parent separately so an OS-level dialog close does not
        // prevent the main list refreshing once an already confirmed import ends.
        let parent = self.parent_main_window.clone();
        cx.notify();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::run_limited_blocking(move || {
                let counts = runtime.add_local_projects(&paths)?;
                let projects = runtime.reload_project_list();
                let existing = find_existing_candidates(&candidates, &projects, &support_dir);
                Ok::<_, String>((counts, projects, existing))
            })
            .await
            .unwrap_or_else(|error| Err(format!("Import failed: {error}")));
            if let Ok((_, projects, _)) = &result
                && let Some(parent) = parent.as_ref().and_then(|parent| parent.upgrade())
            {
                let projects = projects.clone();
                let _ = parent.update(cx, |main, cx| {
                    main.state.projects = projects;
                    main.sync_project_list_state(cx);
                    main.invalidate_project_management(cx);
                });
            }
            let _ = this.update(cx, |app, cx| {
                let Some(batch) = app.project_batch_import.as_mut() else {
                    return;
                };
                batch.importing = false;
                match result {
                    Ok((counts, projects, existing)) => {
                        app.state.projects = projects;
                        batch.existing = existing;
                        batch.selected.clear();
                        batch.summary = Some(counts);
                    }
                    Err(error) => batch.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn project_batch_import_workspace(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let batch = self
            .project_batch_import
            .as_ref()
            .expect("batch workspace needs state");
        let locale = locale_from_language_setting(&self.state.settings.language);
        let tr = |key: &str, fallback: &str| translate(&locale, key, fallback);
        let running = batch.cancel.is_some();
        let search = window.use_keyed_state("batch-project-search", cx, |window, cx| {
            InputState::new(window, cx).placeholder(tr(
                "project.batch.search",
                "Search by name or relative path",
            ))
        });
        let filter = search.read(cx).value().trim().to_lowercase();
        let rows: Vec<_> = batch
            .candidates
            .iter()
            .filter(|project| {
                filter.is_empty()
                    || project.name.to_lowercase().contains(&filter)
                    || project.relative_path.to_lowercase().contains(&filter)
            })
            .map(|project| {
                let path = project.path.clone();
                let existing = batch.existing.get(&path).copied();
                let selected = batch.selected.contains(&path);
                let mut label = match existing {
                    Some(true) => tr("project.batch.hidden", "Already added · Hidden"),
                    Some(false) => tr("project.batch.existing", "Already added"),
                    None => String::new(),
                };
                if matches!(project.kind, ProjectKind::LinkedCheckout) {
                    if !label.is_empty() {
                        label.push_str(" · ");
                    }
                    label.push_str(&tr("project.batch.linked", "Linked checkout"));
                }
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .px(px(5.))
                    .py(px(4.))
                    .child(
                        Checkbox::new(SharedString::from(format!(
                            "batch-project-select:{}",
                            project.relative_path
                        )))
                        .checked(selected)
                        .tooltip(project.name.clone())
                        .disabled(existing.is_some() || batch.importing)
                        .on_click(cx.listener(
                            move |app, checked: &bool, _, cx| {
                                if let Some(batch) = app.project_batch_import.as_mut()
                                    && !batch.importing
                                    && !batch.existing.contains_key(&path)
                                {
                                    if *checked {
                                        batch.selected.insert(path.clone());
                                    } else {
                                        batch.selected.remove(&path);
                                    }
                                }
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().truncate().child(project.name.clone()))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(project.relative_path.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .flex_shrink_0()
                            .child(label),
                    )
            })
            .collect();
        let found = rows.len();
        let total = batch.candidates.len();
        let selected = batch.selected.len();
        let scanning = tr("project.batch.scanning", "Scanning folders…");
        let empty = if total == 0 && batch.root.is_some() {
            tr("project.batch.none_found", "No Git projects found.")
        } else {
            tr("project.batch.no_match", "No matching projects.")
        };
        child_window_shell(tr("project.batch.title", "Import Git projects…"), cx)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .p(px(14.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                dialog_secondary_button(
                                    "batch-folder",
                                    tr("project.batch.choose", "Choose scan folder…"),
                                    cx,
                                    |app, _, window, cx| app.choose_project_batch_root(window, cx),
                                )
                                .disabled(batch.importing),
                            )
                            .when_some(batch.root.as_ref(), |row, path| {
                                row.child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .truncate()
                                        .child(path.to_string_lossy().into_owned()),
                                )
                            })
                            .when(running, |row| {
                                row.child(dialog_cancel_button(
                                    "batch-cancel-scan",
                                    tr("project.batch.cancel_scan", "Cancel scan"),
                                    cx,
                                    |app, _, _, cx| app.cancel_project_batch_scan(cx),
                                ))
                            }),
                    )
                    .when(running, |body| body.child(scanning))
                    .when_some(batch.error.clone(), |body, error| {
                        body.child(div().text_sm().child(error))
                    })
                    .when_some(batch.summary, |body, (added, skipped)| {
                        body.child(
                            tr("project.batch.summary", "Added {added}; skipped {skipped}.")
                                .replace("{added}", &added.to_string())
                                .replace("{skipped}", &skipped.to_string()),
                        )
                    })
                    .when(batch.skipped > 0, |body| {
                        body.child(
                            tr(
                                "project.batch.skipped",
                                "Skipped {count} unreadable entries.",
                            )
                            .replace("{count}", &batch.skipped.to_string()),
                        )
                    })
                    .when(total > 0, |body| {
                        body.child(Input::new(&search).with_size(gpui_component::Size::Medium))
                    })
                    .when(
                        !running && batch.error.is_none() && batch.root.is_some() && found == 0,
                        |body| body.child(empty),
                    )
                    .child(
                        div()
                            .id("batch-project-results")
                            .min_h_0()
                            .flex_1()
                            .overflow_y_scrollbar()
                            .children(rows),
                    ),
            )
            .child(dialog_footer_bar(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        dialog_cancel_button(
                            "batch-back",
                            tr("project.batch.back", "Back"),
                            cx,
                            |app, _, _, cx| {
                                app.project_batch_import = None;
                                cx.notify();
                            },
                        )
                        .disabled(batch.importing),
                    )
                    .child(
                        dialog_primary_button(
                            "batch-import-selected",
                            tr("project.batch.add_selected", "Add selected ({count})")
                                .replace("{count}", &selected.to_string()),
                            cx,
                            |app, _, _, cx| app.accept_project_batch_import(cx),
                        )
                        .disabled(selected == 0 || running || batch.importing)
                        .loading(batch.importing),
                    ),
                cx,
            ))
            .into_any_element()
    }
}

fn existing_candidates(
    result: &ScanResult,
    projects: &[ProjectInfo],
    support_dir: &Path,
) -> BTreeMap<PathBuf, bool> {
    find_existing_candidates(&result.projects, projects, support_dir)
}

fn find_existing_candidates(
    candidates: &[ScannedProject],
    projects: &[ProjectInfo],
    support_dir: &Path,
) -> BTreeMap<PathBuf, bool> {
    let hidden = super::project_column_actions::load_hidden_projects(support_dir);
    use codux_runtime::path::local_path_identity_key;
    let known = projects
        .iter()
        .filter(|project| matches!(project.runtime_target, ProjectRuntimeTarget::Local))
        .filter_map(|project| {
            local_path_identity_key(Path::new(&project.path))
                .map(|key| (key, hidden.contains(&project.id)))
        })
        .collect::<BTreeMap<_, _>>();
    candidates
        .iter()
        .filter_map(|candidate| {
            let key = local_path_identity_key(&candidate.path)?;
            known
                .get(&key)
                .map(|hidden| (candidate.path.clone(), *hidden))
        })
        .collect()
}
