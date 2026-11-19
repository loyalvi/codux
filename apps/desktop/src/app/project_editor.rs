use super::*;
use crate::app::app_state::RemoteBrowseEntry;
use gpui::Focusable;
use gpui_component::input::{Input, InputEvent, InputState, SelectAll};

impl CoduxApp {
    pub(in crate::app) fn project_editor_workspace(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let language = self.state.settings.language.as_str();
        let locale = locale_from_language_setting(language);
        let tr = |key: &str, fallback: &str| translate(&locale, key, fallback);
        let is_editing = self.project_editor_project_id.is_some();
        let title = if is_editing {
            tr("project.edit.title", "Edit Project")
        } else {
            tr("project.create.title", "Create Project")
        };
        let submit_label = if is_editing {
            tr("common.save", "Save")
        } else {
            tr("common.create", "Create")
        };
        let can_submit = !self.project_editor_saving
            && !self.project_editor_name.trim().is_empty()
            && !self.project_editor_path.trim().is_empty();

        child_window_shell(title, cx)
            .child(
                div()
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .flex()
                    .flex_col()
                    .p(px(18.0))
                    .child(project_editor_field(
                        tr("project.editor.name", "Project Name"),
                        "project-editor-name",
                        &self.project_editor_name,
                        "Project",
                        window,
                        cx,
                        |app, value, window, cx| app.set_project_editor_name(value, window, cx),
                    ))
                    .child({
                        let (device_label, is_hosted) = match &self.project_editor_runtime_target {
                            ProjectRuntimeTarget::Local => {
                                (tr("project.editor.device.local", "Local"), false)
                            }
                            ProjectRuntimeTarget::Wsl { distribution } => {
                                (format!("WSL · {distribution}"), true)
                            }
                            ProjectRuntimeTarget::Remote { device_id } => (
                                self.runtime_service
                                    .saved_remote_hosts()
                                    .into_iter()
                                    .find(|host| &host.device_id == device_id)
                                    .map(|host| {
                                        if host.host_name.trim().is_empty() {
                                            host.host_id
                                        } else {
                                            host.host_name
                                        }
                                    })
                                    .unwrap_or_else(|| {
                                        tr("project.editor.device.remote", "Device")
                                    }),
                                true,
                            ),
                        };
                        project_editor_path_field(
                            tr("project.editor.directory", "Project Directory"),
                            tr("project.editor.choose_directory.prompt", "Choose"),
                            &self.project_editor_path,
                            device_label,
                            is_hosted,
                            window,
                            cx,
                        )
                    })
                    .child(project_editor_symbol_field(
                        tr("project.editor.icon", "Project Icon"),
                        tr("common.none", "None"),
                        self.project_editor_badge_symbol.as_deref(),
                        &self.project_editor_badge_color_hex,
                        cx,
                    ))
                    .child(project_editor_color_field(
                        tr("project.editor.color", "Project Color"),
                        &self.project_editor_badge_color_hex,
                        cx,
                    ))
                    .when(is_editing, |panel| {
                        let has_icon =
                            self.project_editor_project_id.as_deref().is_some_and(|id| {
                                custom_project_icon_path(&self.state.support_dir, id).is_some()
                            });
                        panel.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(tr(
                                    "project.editor.image.note",
                                    "Project image (applies immediately)",
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(dialog_primary_button(
                                            "project-editor-choose-image",
                                            tr("project.editor.image.choose", "Choose PNG or SVG…"),
                                            cx,
                                            |app, _, window, cx| app.pick_project_icon(window, cx),
                                        ))
                                        .child(
                                            dialog_cancel_button(
                                                "project-editor-reset-image",
                                                tr("project.editor.image.reset", "Reset image"),
                                                cx,
                                                |app, _, _, cx| app.apply_project_icon(None, cx),
                                            )
                                            .disabled(!has_icon),
                                        ),
                                )
                                .when(self.status_message.starts_with("Project image"), |row| {
                                    row.child(self.status_message.clone())
                                }),
                        )
                    })
                    .child(project_editor_environment_field(
                        tr("project.editor.environment", "Environment Variables"),
                        tr("project.editor.environment.add", "Add Variable"),
                        tr("project.editor.environment.key", "Name"),
                        tr("project.editor.environment.value", "Value"),
                        tr("project.editor.environment.remove", "Remove"),
                        &self.project_editor_environment_variables,
                        window,
                        cx,
                    )),
            )
            .child(dialog_footer_bar(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(dialog_cancel_button(
                        "project-editor-cancel",
                        tr("common.cancel", "Cancel"),
                        cx,
                        |_app, _event, window, _cx| {
                            window.remove_window();
                        },
                    ))
                    .child(
                        dialog_primary_button(
                            "project-editor-save",
                            submit_label,
                            cx,
                            |app, _event, window, cx| {
                                app.save_project_editor(window, cx);
                            },
                        )
                        .disabled(!can_submit)
                        .loading(self.project_editor_saving),
                    ),
                cx,
            ))
    }
}

impl CoduxApp {
    /// The file-picker sub-window: a standard child window (shared title bar via
    /// `child_window_shell`, shared `dialog_footer_bar`) for browsing a local or
    /// remote-host directory and picking a folder. The chosen path is pushed back
    /// to the project-editor window (the opener).
    pub(in crate::app) fn file_picker_window(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let locale = locale_from_language_setting(self.state.settings.language.as_str());
        let tr = |key: &str, fallback: &str| translate(&locale, key, fallback);
        let mode = self.file_picker_mode;
        let title = match mode {
            FilePickerMode::OpenFolder => tr("project.editor.browse.title", "Choose Folder"),
            FilePickerMode::OpenFile => tr("file.picker.open.title", "Open File"),
            FilePickerMode::Save => tr("file.picker.save.title", "Save As"),
        };
        let confirm_label = match mode {
            FilePickerMode::OpenFolder => tr("project.editor.browse.use", "Use this folder"),
            FilePickerMode::OpenFile => tr("file.picker.open.confirm", "Open"),
            FilePickerMode::Save => tr("file.picker.save.confirm", "Save"),
        };
        let current = self.project_editor_browse_path.clone();
        let can_confirm =
            self.file_picker_result_path().is_some() && !self.project_editor_browse_busy;

        // Left: the device sidebar (Local + each paired host). Clicking a
        // device re-lists from its root.
        let active_target = self.project_editor_runtime_target.clone();
        let mut devices = div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .p(px(8.0))
            .overflow_y_scrollbar()
            .child(file_picker_device_row(
                "file-picker-device-local".to_string(),
                tr("project.editor.device.local", "Local"),
                active_target == ProjectRuntimeTarget::Local,
                cx,
                |app, window, cx| {
                    app.file_picker_switch_runtime(ProjectRuntimeTarget::Local, window, cx)
                },
            ));
        for distribution in self.runtime_service.wsl_distributions().unwrap_or_default() {
            let runtime_target = ProjectRuntimeTarget::Wsl {
                distribution: distribution.name.clone(),
            };
            let selected = active_target == runtime_target;
            let target = runtime_target.clone();
            devices = devices.child(file_picker_wsl_device_row(
                distribution.name,
                selected,
                cx,
                move |app, window, cx| app.file_picker_switch_runtime(target.clone(), window, cx),
            ));
        }
        for host in self.runtime_service.saved_remote_hosts() {
            let device_id = host.device_id.clone();
            let selected = active_target
                == ProjectRuntimeTarget::Remote {
                    device_id: host.device_id.clone(),
                };
            let label = if host.host_name.trim().is_empty() {
                host.host_id.clone()
            } else {
                host.host_name.clone()
            };
            devices = devices.child(file_picker_device_row(
                format!("file-picker-device-{}", host.device_id),
                label,
                selected,
                cx,
                move |app, window, cx| {
                    app.file_picker_switch_runtime(
                        ProjectRuntimeTarget::Remote {
                            device_id: device_id.clone(),
                        },
                        window,
                        cx,
                    )
                },
            ));
        }

        // Right: breadcrumb + listing + new-folder/filename rows. The listing
        // lives in a separately constrained scroll body so it cannot grow under
        // the fixed dialog footer when a folder has many entries.
        let mut list = div()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(16.0))
            .pt(px(10.0))
            .pb(px(12.0));
        let visible_entries = self.project_editor_browse_entries.len()
            + usize::from(self.project_editor_browse_parent.is_some())
            + usize::from(self.file_picker_new_folder_active);
        let entry_labels = FilePickerEntryLabels {
            open: tr("common.open", "Open"),
            choose: tr("common.choose", "Choose"),
            rename: tr("common.rename", "Rename"),
            delete: tr("common.delete", "Delete"),
        };
        if let Some(parent) = self.project_editor_browse_parent.clone() {
            list = list.child(file_picker_entry_row(
                RemoteBrowseEntry {
                    name: "..".to_string(),
                    path: parent.clone(),
                    is_dir: true,
                },
                self.file_picker_active_path.as_deref() == Some(parent.as_str()),
                mode,
                &entry_labels,
                cx,
                false,
            ));
        }
        if self.project_editor_browse_busy && visible_entries == 0 {
            list = list.child(file_picker_status_state(
                HeroIconName::ArrowPath,
                tr("file.picker.loading", "Loading folder…"),
                true,
                cx,
            ));
        } else if !self.project_editor_browse_busy
            && visible_entries == 0
            && self.project_editor_browse_error.is_none()
        {
            list = list.child(file_picker_status_state(
                HeroIconName::FolderOpen,
                tr("file.picker.empty", "This folder is empty."),
                false,
                cx,
            ));
        }
        if self.file_picker_new_folder_active {
            list = list.child(file_picker_new_folder_row(
                &self.project_editor_browse_new_folder,
                tr(
                    "project.editor.browse.new_folder_placeholder",
                    "New folder name",
                ),
                window,
                cx,
            ));
        }
        for entry in &self.project_editor_browse_entries {
            if self
                .file_picker_rename_draft
                .as_ref()
                .is_some_and(|draft| draft.path == entry.path)
            {
                list = list.child(file_picker_rename_row(
                    entry.is_dir,
                    self.file_picker_rename_draft
                        .as_ref()
                        .map(|draft| draft.name.as_str())
                        .unwrap_or(entry.name.as_str()),
                    tr("file.picker.rename.placeholder", "New name"),
                    window,
                    cx,
                ));
            } else {
                let selected = self.file_picker_active_path.as_deref() == Some(entry.path.as_str())
                    || (!entry.is_dir
                        && self.file_picker_selected.as_deref() == Some(entry.path.as_str()));
                list = list.child(file_picker_entry_row(
                    entry.clone(),
                    selected,
                    mode,
                    &entry_labels,
                    cx,
                    true,
                ));
            }
        }

        let root_label = match &active_target {
            ProjectRuntimeTarget::Local => tr("project.editor.device.local", "Local"),
            ProjectRuntimeTarget::Wsl { distribution } => format!("WSL · {distribution}"),
            ProjectRuntimeTarget::Remote { device_id } => self
                .runtime_service
                .saved_remote_hosts()
                .into_iter()
                .find(|host| &host.device_id == device_id)
                .map(|host| {
                    if host.host_name.trim().is_empty() {
                        host.host_id
                    } else {
                        host.host_name
                    }
                })
                .unwrap_or_else(|| tr("project.editor.device.remote", "Device")),
        };
        let mut right = div()
            .min_h_0()
            .flex_1()
            .flex_basis(px(0.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(color(theme::BORDER_SOFT))
                    .child(
                        gpui_component::scroll::ScrollableElement::overflow_x_scrollbar(
                            div().flex_1().min_w_0(),
                        )
                        .child(file_picker_breadcrumb(
                            &current,
                            &root_label,
                            active_target.is_hosted(),
                            cx,
                        )),
                    )
                    .child(
                        div()
                            .flex_none()
                            .px(px(12.0))
                            .child(file_picker_refresh_button(
                                self.project_editor_browse_busy,
                                tr("common.refresh", "Refresh"),
                                cx,
                            )),
                    ),
            )
            .child(
                crate::app::scroll_compat::ScrollableElement::overflow_y_scrollbar(
                    div().min_h_0().flex_1().flex_basis(px(0.0)),
                )
                .child(list),
            );
        // Save mode: a filename row (prefilled when an existing file is clicked).
        if mode == FilePickerMode::Save {
            right = right.child(div().flex_none().min_w_0().px(px(16.0)).pb(px(12.0)).child(
                project_editor_input(
                    "file-picker-filename",
                    &self.file_picker_filename,
                    "File name",
                    window,
                    cx,
                    |app, value, window, cx| app.set_file_picker_filename(value, window, cx),
                ),
            ));
        }
        if let Some(error) = self.project_editor_browse_error.as_ref() {
            right = right.child(
                div()
                    .flex_none()
                    .px(px(16.0))
                    .pb(px(12.0))
                    .text_size(rems(0.8125))
                    .text_color(color(theme::ORANGE))
                    .child(error.clone()),
            );
        }

        let body = div().min_h_0().flex_1().overflow_hidden().flex().child(
            h_resizable("file-picker-device-split")
                .child(
                    resizable_panel()
                        .size(px(180.0))
                        .size_range(px(140.0)..px(280.0))
                        .child(devices),
                )
                .child(
                    resizable_panel()
                        .size_range(px(320.0)..Pixels::MAX)
                        .child(right),
                ),
        );

        let new_folder_disabled =
            self.project_editor_browse_busy || self.project_editor_browse_path.trim().is_empty();
        child_window_shell(SharedString::from(title), cx)
            .child(body)
            .child(dialog_footer_bar(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        Button::new("file-picker-new-folder")
                            .secondary()
                            .compact()
                            .icon(Icon::new(HeroIconName::FolderPlus).size_3p5())
                            .child(dialog_button_label(tr(
                                "project.editor.browse.new_folder",
                                "New folder",
                            )))
                            .disabled(new_folder_disabled)
                            .on_click(cx.listener(|app, _event, _window, cx| {
                                app.begin_file_picker_new_folder(cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(dialog_cancel_button(
                                "file-picker-cancel",
                                tr("common.cancel", "Cancel"),
                                cx,
                                |_app, _event, window, _cx| {
                                    window.remove_window();
                                },
                            ))
                            .child(
                                dialog_primary_button(
                                    "file-picker-use",
                                    confirm_label,
                                    cx,
                                    |app, _event, window, cx| app.file_picker_select(window, cx),
                                )
                                .disabled(!can_confirm),
                            ),
                    ),
                cx,
            ))
            .into_any_element()
    }
}

/// The inline new-folder name editor shown in the listing: a folder icon + a
/// text field that commits on Enter (or blur when non-empty) and dismisses on
/// blur when left empty.
fn file_picker_new_folder_row(
    value: &str,
    placeholder: String,
    window: &mut Window,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let value = value.to_string();
    let input_state =
        window.use_keyed_state(SharedString::from("file-picker-newfolder-inline"), cx, {
            let placeholder = placeholder.clone();
            move |window, cx| InputState::new(window, cx).placeholder(placeholder.clone())
        });
    input_state.update(cx, |state, cx| {
        if !state.focus_handle(cx).is_focused(window) {
            if state.value().as_ref() != value {
                state.set_value(value.clone(), window, cx);
            }
            state.focus(window, cx);
            if state.selected_range().is_empty() {
                window.dispatch_action(Box::new(SelectAll), cx);
            }
        }
    });
    cx.subscribe_in(
        &input_state,
        window,
        move |app, state, event, window, cx| match event {
            InputEvent::Change => app.set_project_editor_browse_new_folder(
                state.read(cx).value().to_string(),
                window,
                cx,
            ),
            InputEvent::PressEnter { .. } => app.project_editor_browse_create_folder(window, cx),
            InputEvent::Blur => {
                if app.project_editor_browse_new_folder.trim().is_empty() {
                    app.cancel_file_picker_new_folder(cx);
                } else {
                    app.project_editor_browse_create_folder(window, cx);
                }
            }
            _ => {}
        },
    )
    .detach();

    div()
        .flex()
        .items_center()
        .gap_2()
        .px(px(8.0))
        .py(px(4.0))
        .child(
            Icon::new(HeroIconName::Folder)
                .size_4()
                .text_color(color(theme::ACCENT)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&input_state).with_size(gpui_component::Size::Small)),
        )
        .into_any_element()
}

fn file_picker_rename_row(
    is_dir: bool,
    value: &str,
    placeholder: String,
    window: &mut Window,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let value = value.to_string();
    let input_state =
        window.use_keyed_state(SharedString::from("file-picker-rename-inline"), cx, {
            let placeholder = placeholder.clone();
            move |window, cx| InputState::new(window, cx).placeholder(placeholder.clone())
        });
    input_state.update(cx, |state, cx| {
        if !state.focus_handle(cx).is_focused(window) {
            if state.value().as_ref() != value {
                state.set_value(value.clone(), window, cx);
            }
            state.focus(window, cx);
            if state.selected_range().is_empty() {
                window.dispatch_action(Box::new(SelectAll), cx);
            }
        }
    });
    cx.subscribe_in(
        &input_state,
        window,
        move |app, state, event, window, cx| match event {
            InputEvent::Change => {
                app.set_file_picker_rename_name(state.read(cx).value().to_string(), window, cx)
            }
            InputEvent::PressEnter { .. } => app.confirm_file_picker_rename(window, cx),
            InputEvent::Blur => {
                if app
                    .file_picker_rename_draft
                    .as_ref()
                    .is_some_and(|draft| draft.name.trim().is_empty())
                {
                    app.cancel_file_picker_rename(cx);
                } else {
                    app.confirm_file_picker_rename(window, cx);
                }
            }
            _ => {}
        },
    )
    .detach();

    div()
        .flex()
        .items_center()
        .gap_2()
        .px(px(8.0))
        .py(px(4.0))
        .child(
            Icon::new(if is_dir {
                HeroIconName::Folder
            } else {
                HeroIconName::Document
            })
            .size_4()
            .text_color(color(if is_dir {
                theme::ACCENT
            } else {
                theme::TEXT_MUTED
            })),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&input_state).with_size(gpui_component::Size::Small)),
        )
        .into_any_element()
}

struct FilePickerEntryLabels {
    open: String,
    choose: String,
    rename: String,
    delete: String,
}

fn file_picker_entry_row(
    entry: RemoteBrowseEntry,
    selected: bool,
    mode: FilePickerMode,
    labels: &FilePickerEntryLabels,
    cx: &mut Context<CoduxApp>,
    allow_mutations: bool,
) -> AnyElement {
    let open_label = labels.open.clone();
    let choose_label = labels.choose.clone();
    let rename_label = labels.rename.clone();
    let delete_label = labels.delete.clone();
    let id = format!("file-picker-{}", entry.path);
    let label = entry.name.clone();
    let is_dir = entry.is_dir;
    let click_path = entry.path.clone();
    let right_click_entry = entry.clone();
    let mut row = div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap_2()
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().secondary_hover))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |app, _event, _window, cx| {
                app.select_file_picker_context_entry(right_click_entry.clone(), cx);
            }),
        )
        .on_click(cx.listener(move |app, _event, window, cx| {
            app.file_picker_choose_entry(click_path.clone(), is_dir, window, cx)
        }))
        .child(
            Icon::new(if is_dir {
                HeroIconName::Folder
            } else {
                HeroIconName::Document
            })
            .size_4()
            .text_color(color(if is_dir {
                theme::ACCENT
            } else {
                theme::TEXT_MUTED
            })),
        )
        .child(
            div()
                .text_size(rems(0.8125))
                .text_color(color(theme::TEXT))
                .truncate()
                .child(label),
        );
    if selected {
        row = row.bg(cx.theme().secondary);
    }
    let app_entity = cx.entity();
    row.context_menu(move |menu, _window, _cx| {
        let open_entity = app_entity.clone();
        let rename_entity = app_entity.clone();
        let delete_entity = app_entity.clone();
        let open_entry = entry.clone();
        let rename_entry = entry.clone();
        let delete_entry = entry.clone();
        let primary_label = if is_dir || mode == FilePickerMode::OpenFile {
            open_label.clone()
        } else {
            choose_label.clone()
        };
        let menu = menu.item(
            PopupMenuItem::new(primary_label)
                .icon(if is_dir {
                    HeroIconName::FolderOpen
                } else {
                    HeroIconName::Check
                })
                .on_click(move |_, window, cx| {
                    cx.update_entity(&open_entity, |app: &mut CoduxApp, cx| {
                        app.file_picker_choose_entry(
                            open_entry.path.clone(),
                            open_entry.is_dir,
                            window,
                            cx,
                        );
                    });
                }),
        );
        if !allow_mutations {
            return menu;
        }
        menu.separator()
            .item(
                PopupMenuItem::new(rename_label.clone())
                    .icon(HeroIconName::PencilSquare)
                    .on_click(move |_, _window, cx| {
                        cx.update_entity(&rename_entity, |app: &mut CoduxApp, cx| {
                            app.start_file_picker_rename(rename_entry.clone(), cx);
                        });
                    }),
            )
            .item(
                PopupMenuItem::new(delete_label.clone())
                    .icon(HeroIconName::Trash)
                    .on_click(move |_, _window, cx| {
                        cx.update_entity(&delete_entity, |app: &mut CoduxApp, cx| {
                            app.request_delete_file_picker_entry(delete_entry.clone(), cx);
                        });
                    }),
            )
    })
    .into_any_element()
}

fn file_picker_status_state(
    icon: HeroIconName,
    label: String,
    loading: bool,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    div()
        .min_h(px(180.0))
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .text_color(cx.theme().muted_foreground)
        .child(
            div()
                .size(px(32.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.0))
                .bg(cx.theme().secondary.opacity(0.6))
                .child(if loading {
                    Spinner::new()
                        .small()
                        .color(cx.theme().muted_foreground)
                        .into_any_element()
                } else {
                    Icon::new(icon)
                        .size_4()
                        .text_color(cx.theme().muted_foreground)
                        .into_any_element()
                }),
        )
        .child(
            div()
                .text_size(rems(0.8125))
                .line_height(rems(1.125))
                .child(label),
        )
        .into_any_element()
}

/// A device row in the file picker's left sidebar (This Mac / a host).
fn file_picker_device_row(
    id: String,
    label: String,
    selected: bool,
    cx: &mut Context<CoduxApp>,
    on_click: impl Fn(&mut CoduxApp, &mut Window, &mut Context<CoduxApp>) + 'static,
) -> AnyElement {
    let mut row = div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap_2()
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .text_color(color(theme::TEXT))
        // Hover tints the text (and icon, via inheritance) the theme accent;
        // background is reserved for the selected state only.
        .hover(|style| style.text_color(cx.theme().primary))
        .child(Icon::new(HeroIconName::GlobeAlt).size_3())
        .child(div().text_size(rems(0.8125)).truncate().child(label))
        .on_click(cx.listener(move |app, _event, window, cx| on_click(app, window, cx)));
    if selected {
        row = row.bg(cx.theme().secondary);
    }
    row.into_any_element()
}

fn file_picker_wsl_device_row(
    distribution: String,
    selected: bool,
    cx: &mut Context<CoduxApp>,
    on_select: impl Fn(&mut CoduxApp, &mut Window, &mut Context<CoduxApp>) + 'static,
) -> AnyElement {
    let id = format!("file-picker-wsl-{distribution}");
    let mut row = div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap_2()
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .text_color(color(theme::TEXT))
        .hover(|style| style.text_color(cx.theme().primary))
        .child(Icon::new(HeroIconName::CommandLine).size_3())
        .child(
            div()
                .h(px(18.0))
                .px(px(5.0))
                .rounded(px(4.0))
                .flex_none()
                .flex()
                .items_center()
                .text_size(rems(0.625))
                .line_height(rems(0.875))
                .font_weight(FontWeight::MEDIUM)
                .bg(color(theme::ACCENT).opacity(0.12))
                .text_color(color(theme::ACCENT))
                .child("WSL"),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_size(rems(0.8125))
                .truncate()
                .child(distribution),
        )
        .on_click(cx.listener(move |app, _event, window, cx| on_select(app, window, cx)));
    if selected {
        row = row.bg(cx.theme().secondary);
    }
    row.into_any_element()
}

/// A Finder-style path bar for the current directory: a device root crumb
/// (computer/server icon + label) followed by chevron-separated, clickable path
/// segments. Sits under a bottom border to set it off from the listing.
fn file_picker_breadcrumb(
    path: &str,
    root_label: &str,
    is_remote: bool,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let bar = div()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .gap(px(2.0))
        .px(px(16.0))
        .pt(px(16.0))
        .pb(px(10.0));
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return bar
            .child(
                div()
                    .text_size(rems(0.8125))
                    .text_color(color(theme::TEXT_MUTED))
                    .child("Loading…"),
            )
            .into_any_element();
    }
    // The root crumb stands in for the bare filesystem root: an icon + the
    // device label (e.g. "Local" / a host name) navigating to the root target
    // (`/` on POSIX, the drive list on Windows).
    let (root_target, segments) = file_picker_breadcrumb_model(trimmed);
    let mut row = bar.child(file_picker_root_crumb(
        root_label,
        &root_target,
        is_remote,
        cx,
    ));
    for segment in segments {
        row = row.child(file_picker_crumb_separator());
        row = row.child(file_picker_crumb(
            &format!("file-picker-crumb-{}", segment.target),
            &segment.label,
            &segment.target,
            cx,
        ));
    }
    row.into_any_element()
}

/// Re-list the current directory. Remote browsing never auto-refreshes (the
/// watcher is host-side), so this is the manual reload in the footer actions.
fn file_picker_refresh_button(
    loading: bool,
    tooltip: String,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    Button::new("file-picker-refresh")
        .ghost()
        .compact()
        .with_size(Size::Small)
        .icon(Icon::new(HeroIconName::ArrowPath).size_3p5())
        .tooltip(tooltip)
        .loading(loading)
        .disabled(loading)
        .on_click(cx.listener(|app, _event, window, cx| {
            let current = app.project_editor_browse_path.clone();
            if !current.trim().is_empty() {
                app.project_editor_browse_navigate(Some(current), window, cx);
            }
        }))
        .into_any_element()
}

/// One clickable breadcrumb segment: its visible label and the absolute path it
/// navigates to.
struct FilePickerCrumb {
    label: String,
    target: String,
}

/// Adapt the shared, typed-path-backed [`codux_runtime::path::breadcrumb_segments`]
/// splitter into the picker's crumb type. Splitting lives in the core crate so
/// Windows / POSIX / UNC / drive-list paths are all parsed the same way wherever
/// they're shown.
fn file_picker_breadcrumb_model(path: &str) -> (String, Vec<FilePickerCrumb>) {
    let (root_target, segments) = codux_runtime::path::breadcrumb_segments(path);
    let crumbs = segments
        .into_iter()
        .map(|(label, target)| FilePickerCrumb { label, target })
        .collect();
    (root_target, crumbs)
}

/// The `›` chevron between path segments.
fn file_picker_crumb_separator() -> AnyElement {
    Icon::new(HeroIconName::ChevronRight)
        .size_3()
        .flex_shrink_0()
        .text_color(color(theme::TEXT_DIM))
        .into_any_element()
}

/// The leading device crumb (icon + label) that navigates to the root target
/// (`/` on POSIX, the drive list on Windows).
fn file_picker_root_crumb(
    label: &str,
    root_target: &str,
    is_remote: bool,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let root_target = root_target.to_string();
    div()
        .id("file-picker-crumb-root")
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(5.0))
        .px(px(6.0))
        .py(px(2.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().secondary))
        .child(
            Icon::new(if is_remote {
                HeroIconName::ServerStack
            } else {
                HeroIconName::ComputerDesktop
            })
            .size_4()
            .text_color(color(theme::TEXT_MUTED)),
        )
        .child(
            div()
                .text_size(rems(0.8125))
                .font_weight(FontWeight::MEDIUM)
                .text_color(color(theme::TEXT))
                .child(label.to_string()),
        )
        .on_click(cx.listener(move |app, _event, window, cx| {
            app.project_editor_browse_navigate(Some(root_target.clone()), window, cx)
        }))
        .into_any_element()
}

fn file_picker_crumb(
    id: &str,
    label: &str,
    target: &str,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let target = target.to_string();
    div()
        .id(SharedString::from(id.to_string()))
        .flex_shrink_0()
        .px(px(6.0))
        .py(px(2.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().secondary))
        .text_size(rems(0.8125))
        .text_color(color(theme::TEXT))
        .child(label.to_string())
        .on_click(cx.listener(move |app, _event, window, cx| {
            app.project_editor_browse_navigate(Some(target.clone()), window, cx)
        }))
        .into_any_element()
}

struct ProjectEditorSymbol {
    id: &'static str,
    icon: Option<HeroIconName>,
}

const PROJECT_EDITOR_SYMBOLS: &[ProjectEditorSymbol] = &[
    ProjectEditorSymbol {
        id: "none",
        icon: None,
    },
    ProjectEditorSymbol {
        id: "terminal",
        icon: Some(HeroIconName::CommandLine),
    },
    ProjectEditorSymbol {
        id: "folder",
        icon: Some(HeroIconName::Folder),
    },
    ProjectEditorSymbol {
        id: "shippingbox",
        icon: Some(HeroIconName::Sparkles),
    },
    ProjectEditorSymbol {
        id: "hammer",
        icon: Some(HeroIconName::WrenchScrewdriver),
    },
    ProjectEditorSymbol {
        id: "server.rack",
        icon: Some(HeroIconName::GlobeAlt),
    },
    ProjectEditorSymbol {
        id: "globe",
        icon: Some(HeroIconName::GlobeAlt),
    },
    ProjectEditorSymbol {
        id: "bolt",
        icon: Some(HeroIconName::Star),
    },
    ProjectEditorSymbol {
        id: "wrench",
        icon: Some(HeroIconName::Cog6Tooth),
    },
    ProjectEditorSymbol {
        id: "doc.text",
        icon: Some(HeroIconName::Document),
    },
    ProjectEditorSymbol {
        id: "book",
        icon: Some(HeroIconName::BookOpen),
    },
    ProjectEditorSymbol {
        id: "person.2",
        icon: Some(HeroIconName::UserCircle),
    },
];

fn project_editor_field(
    label: String,
    id: &'static str,
    value: &str,
    placeholder: impl Into<String>,
    window: &mut Window,
    cx: &mut Context<CoduxApp>,
    action: impl Fn(&mut CoduxApp, String, &mut Window, &mut Context<CoduxApp>) + 'static,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .mb(px(24.0))
        .child(
            div()
                .text_size(rems(0.875))
                .line_height(rems(1.125))
                .text_color(color(theme::TEXT))
                .child(label),
        )
        .child(project_editor_input(
            id,
            value,
            placeholder,
            window,
            cx,
            action,
        ))
        .into_any_element()
}

fn project_editor_symbol_field(
    label: String,
    none_label: String,
    selected_symbol: Option<&str>,
    selected_color: &str,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let accent = hex_color(selected_color).unwrap_or(theme::ACCENT);
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .mb(px(24.0))
        .child(
            div()
                .text_size(rems(0.875))
                .line_height(rems(1.125))
                .text_color(color(theme::TEXT))
                .child(label),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .children(PROJECT_EDITOR_SYMBOLS.iter().map(|symbol| {
                    let id = symbol.id;
                    let selected = if id == "none" {
                        selected_symbol.is_none()
                    } else {
                        selected_symbol == Some(id)
                    };
                    div()
                        .id(SharedString::from(format!("project-editor-symbol-{id}")))
                        .size(px(36.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(color(if selected {
                            theme::BORDER
                        } else {
                            theme::BORDER_SOFT
                        }))
                        .bg(if selected {
                            cx.theme().secondary_hover
                        } else {
                            cx.theme().secondary
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().secondary_hover))
                        .on_click(cx.listener(move |app, _event, window, cx| {
                            let next = (id != "none").then(|| id.to_string());
                            app.set_project_editor_badge_symbol(next, window, cx);
                        }))
                        .child(match symbol.icon {
                            Some(icon) => Icon::new(icon)
                                .size_4()
                                .text_color(color(accent))
                                .into_any_element(),
                            None => div()
                                .text_size(rems(0.75))
                                .line_height(rems(1.0))
                                .text_color(color(theme::TEXT_MUTED))
                                .child(none_label.clone())
                                .into_any_element(),
                        })
                        .into_any_element()
                })),
        )
        .into_any_element()
}

fn project_editor_color_field(
    label: String,
    selected_color: &str,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .text_size(rems(0.875))
                .line_height(rems(1.125))
                .text_color(color(theme::TEXT))
                .child(label),
        )
        .child(div().flex().flex_wrap().items_center().gap_3().children(
            PROJECT_BADGE_COLORS.iter().map(|value| {
                let selected = *value == selected_color;
                let swatch = hex_color(value).unwrap_or(theme::ACCENT);
                div()
                    .id(SharedString::from(format!("project-editor-color-{value}")))
                    .size(px(24.0))
                    .rounded_full()
                    .bg(color(swatch))
                    .border_1()
                    .border_color(color(if selected {
                        theme::TEXT
                    } else {
                        theme::BORDER_SOFT
                    }))
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.86))
                    .on_click(cx.listener(move |app, _event, window, cx| {
                        app.set_project_editor_badge_color((*value).to_string(), window, cx);
                    }))
                    .into_any_element()
            }),
        ))
        .into_any_element()
}

fn project_editor_environment_field(
    label: String,
    add_label: String,
    key_placeholder: String,
    value_placeholder: String,
    remove_label: String,
    variables: &[ProjectEnvironmentVariableDraft],
    window: &mut Window,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let mut rows = div().flex().flex_col().gap(px(8.0));
    for variable in variables {
        let id = variable.id;
        let key_input_id = SharedString::from(format!("project-editor-env-key-{id}"));
        let value_input_id = SharedString::from(format!("project-editor-env-value-{id}"));
        rows = rows.child(
            div()
                .grid()
                .grid_cols(12)
                .gap(px(8.0))
                .items_center()
                .child(div().col_span(5).child(project_editor_input(
                    key_input_id,
                    &variable.key,
                    key_placeholder.clone(),
                    window,
                    cx,
                    move |app, value, window, cx| {
                        app.set_project_editor_environment_variable_key(id, value, window, cx)
                    },
                )))
                .child(div().col_span(6).child(project_editor_input(
                    value_input_id,
                    &variable.value,
                    value_placeholder.clone(),
                    window,
                    cx,
                    move |app, value, window, cx| {
                        app.set_project_editor_environment_variable_value(id, value, window, cx)
                    },
                )))
                .child(
                    div().col_span(1).flex().justify_end().child(
                        Button::new(SharedString::from(format!(
                            "project-editor-env-remove-{id}"
                        )))
                        .secondary()
                        .compact()
                        .icon(Icon::new(HeroIconName::Trash).size_3p5())
                        .tooltip(remove_label.clone())
                        .on_click(cx.listener(
                            move |app, _event, _window, cx| {
                                app.remove_project_editor_environment_variable(id, cx);
                            },
                        )),
                    ),
                ),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .mt(px(24.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .text_size(rems(0.875))
                        .line_height(rems(1.125))
                        .text_color(color(theme::TEXT))
                        .child(label),
                )
                .child(
                    Button::new("project-editor-env-add")
                        .secondary()
                        .compact()
                        .icon(Icon::new(HeroIconName::Plus).size_3p5())
                        .child(dialog_button_label(add_label))
                        .on_click(cx.listener(|app, _event, _window, cx| {
                            app.add_project_editor_environment_variable(cx);
                        })),
                ),
        )
        .child(rows)
        .into_any_element()
}

fn hex_color(value: &str) -> Option<u32> {
    let value = value.trim().trim_start_matches('#');
    if value.len() == 6 {
        u32::from_str_radix(value, 16).ok()
    } else {
        None
    }
}

fn project_editor_input(
    id: impl Into<SharedString>,
    value: &str,
    placeholder: impl Into<String>,
    window: &mut Window,
    cx: &mut Context<CoduxApp>,
    action: impl Fn(&mut CoduxApp, String, &mut Window, &mut Context<CoduxApp>) + 'static,
) -> AnyElement {
    let id = id.into();
    let value = value.to_string();
    let placeholder = placeholder.into();
    let input_state = window.use_keyed_state(id, cx, |window, cx| {
        InputState::new(window, cx)
            .default_value(value.clone())
            .placeholder(placeholder.clone())
    });
    input_state.update(cx, |state, cx| {
        if state.value().as_ref() != value {
            state.set_value(value.clone(), window, cx);
        }
    });
    cx.subscribe_in(
        &input_state,
        window,
        move |app, state, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                action(app, state.read(cx).value().to_string(), window, cx);
            }
        },
    )
    .detach();

    Input::new(&input_state)
        .with_size(gpui_component::Size::Medium)
        .into_any_element()
}

fn project_editor_path_field(
    label: String,
    choose_label: String,
    path: &str,
    device_label: String,
    is_remote: bool,
    _window: &mut Window,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    // The directory is read-only: it can only be set via the picker (Choose), so
    // the device + path always stay consistent. The whole box is clickable.
    let device_prefix = div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .pr(px(8.0))
        .mr(px(8.0))
        .border_r_1()
        .border_color(color(theme::BORDER_SOFT))
        .child(
            Icon::new(if is_remote {
                HeroIconName::ServerStack
            } else {
                HeroIconName::ComputerDesktop
            })
            .size_4()
            .text_color(color(theme::TEXT_MUTED)),
        )
        .child(
            div()
                .max_w(px(140.0))
                .truncate()
                .text_size(rems(0.8125))
                .text_color(color(theme::TEXT_MUTED))
                .child(device_label),
        );
    let has_path = !path.trim().is_empty();
    let path_text = if has_path {
        path.to_string()
    } else {
        "/path/to/project".to_string()
    };
    let path_box = div()
        .id("project-editor-path")
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .h(px(34.0))
        .px(px(10.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(color(theme::BORDER_SOFT))
        .bg(cx.theme().secondary.opacity(0.4))
        .cursor_pointer()
        .hover(|style| style.border_color(color(theme::BORDER)))
        .on_click(cx.listener(|app, _event, window, cx| {
            app.choose_project_editor_directory(window, cx);
        }))
        .child(device_prefix)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rems(0.8125))
                .text_color(color(if has_path {
                    theme::TEXT
                } else {
                    theme::TEXT_DIM
                }))
                .child(path_text),
        );

    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .mb(px(24.0))
        .child(
            div()
                .text_size(rems(0.875))
                .line_height(rems(1.125))
                .text_color(color(theme::TEXT))
                .child(label),
        )
        .child(
            div().flex().items_center().gap_2().child(path_box).child(
                Button::new("project-editor-choose-path")
                    .secondary()
                    .compact()
                    .text_color(cx.theme().secondary_foreground)
                    .child(dialog_button_label(choose_label))
                    .on_click(cx.listener(|app, _event, window, cx| {
                        app.choose_project_editor_directory(window, cx);
                    })),
            ),
        )
        .into_any_element()
}

#[cfg(test)]
mod breadcrumb_tests {
    use super::file_picker_breadcrumb_model;

    fn crumbs(path: &str) -> (String, Vec<(String, String)>) {
        let (root, segments) = file_picker_breadcrumb_model(path);
        (
            root,
            segments
                .into_iter()
                .map(|segment| (segment.label, segment.target))
                .collect(),
        )
    }

    #[test]
    fn posix_path_splits_on_forward_slash() {
        let (root, segments) = crumbs("/Users/dux/project");
        assert_eq!(root, "/");
        assert_eq!(
            segments,
            vec![
                ("Users".into(), "/Users".into()),
                ("dux".into(), "/Users/dux".into()),
                ("project".into(), "/Users/dux/project".into()),
            ]
        );
    }

    #[test]
    fn windows_drive_path_splits_into_clickable_crumbs() {
        let (root, segments) = crumbs(r"C:\Users\dux");
        assert_eq!(root, codux_runtime::path::FILE_LIST_DRIVES_SENTINEL);
        assert_eq!(
            segments,
            vec![
                ("C:".into(), r"C:\".into()),
                ("Users".into(), r"C:\Users".into()),
                ("dux".into(), r"C:\Users\dux".into()),
            ]
        );
    }

    #[test]
    fn windows_forward_slash_path_also_splits() {
        let (_root, segments) = crumbs("C:/Users/dux");
        let targets: Vec<String> = segments.into_iter().map(|(_, target)| target).collect();
        assert_eq!(targets, vec![r"C:\", r"C:\Users", r"C:\Users\dux"]);
    }

    #[test]
    fn windows_drive_root_shows_only_drive_crumb() {
        let (_root, segments) = crumbs(r"C:\");
        assert_eq!(segments, vec![("C:".into(), r"C:\".into())]);
    }

    #[test]
    fn drive_list_view_has_no_segments() {
        let (root, segments) = crumbs(codux_runtime::path::FILE_LIST_DRIVES_SENTINEL);
        assert_eq!(root, codux_runtime::path::FILE_LIST_DRIVES_SENTINEL);
        assert!(segments.is_empty());
    }
}

fn custom_project_icon_dir(support_dir: &std::path::Path, project_id: &str) -> std::path::PathBuf {
    let key = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, project_id.as_bytes());
    support_dir.join("project-icons").join(key.to_string())
}

pub(in crate::app) fn custom_project_icon_path(
    support_dir: &std::path::Path,
    project_id: &str,
) -> Option<std::path::PathBuf> {
    let dir = custom_project_icon_dir(support_dir, project_id);
    let key = std::fs::read_to_string(dir.join("current")).ok()?;
    let key = uuid::Uuid::parse_str(key.trim()).ok()?;
    let path = dir.join(format!("{key}.png"));
    path.is_file().then_some(path)
}

pub(in crate::app) fn update_custom_project_icon(
    support_dir: &std::path::Path,
    project_id: &str,
    source: Option<&std::path::Path>,
) -> Result<(), String> {
    use std::io::Read as _;
    let dir = custom_project_icon_dir(support_dir, project_id);
    let Some(source) = source else {
        return match std::fs::remove_dir_all(dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    };
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(source)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("The project image exceeds 8 MB.".into());
    }
    let bytes = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(2048);
        limits.max_image_height = Some(2048);
        reader.limits(limits);
        reader
            .decode()
            .map_err(|error| format!("Invalid PNG: {error}"))?;
        bytes
    } else {
        rasterize_project_svg(&bytes)?
    };

    let key = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &bytes);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let destination = dir.join(format!("{key}.png"));
    let previous = custom_project_icon_path(support_dir, project_id);
    if !destination.is_file() {
        std::fs::write(&destination, bytes).map_err(|error| error.to_string())?;
    }
    let draft = dir.join("current.tmp");
    std::fs::write(&draft, key.to_string()).map_err(|error| error.to_string())?;
    std::fs::rename(&draft, dir.join("current")).map_err(|error| error.to_string())?;
    if let Some(previous) = previous.filter(|old| old != &destination) {
        let _ = std::fs::remove_file(previous);
    }
    Ok(())
}

// Rasterize only static SVG icons. Neither SVG bytes nor references are passed
// to the UI renderer, and all persisted project images keep the PNG contract.
fn rasterize_project_svg(bytes: &[u8]) -> Result<Vec<u8>, String> {
    const SVG_NS: &str = "http://www.w3.org/2000/svg";
    const MAX_SVG_BYTES: usize = 256 * 1024;
    const TILE_SIZE: u32 = 256;
    const GLYPH_SIZE: f32 = 190.0;
    if bytes.len() > MAX_SVG_BYTES {
        return Err("The SVG exceeds 256 KB.".into());
    }
    let source =
        std::str::from_utf8(bytes).map_err(|_| "Choose a PNG or a UTF-8 SVG.".to_string())?;
    let doc = roxmltree::Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 4096,
        },
    )
    .map_err(|error| format!("Invalid SVG: {error}"))?;
    let root = doc.root_element();
    if !root.has_tag_name((SVG_NS, "svg")) {
        return Err("Choose a PNG or an SVG document.".into());
    }
    for node in doc.descendants().filter(|node| node.is_element()) {
        let tag = node.tag_name();
        if tag.namespace() != Some(SVG_NS)
            || !matches!(
                tag.name(),
                "svg"
                    | "g"
                    | "defs"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
                    | "linearGradient"
                    | "radialGradient"
                    | "stop"
                    | "clipPath"
                    | "mask"
                    | "title"
                    | "desc"
            )
            || node.ancestors().nth(34).is_some()
        {
            return Err(format!(
                "Unsupported element in a static SVG icon: {}.",
                tag.name()
            ));
        }
        if node.attributes().any(|attribute| {
            attribute.name().starts_with("on")
                || (attribute.name() == "href" && !attribute.value().trim().starts_with('#'))
        }) {
            return Err("Scripts and external SVG resources are not supported.".into());
        }
    }
    // Most outline icons specify currentColor and expect a foreground color
    // from their host. Supply white for a single viewport on the colored tile
    // only when the author did not set color (an inline style still wins).
    let default_color = root.attribute("color").is_none()
        && !root
            .descendants()
            .skip(1)
            .any(|node| node.has_tag_name((SVG_NS, "svg")));
    let options = resvg::usvg::Options {
        style_sheet: default_color.then(|| "svg { color: white; }".to_string()),
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_xmltree(&doc, &options)
        .map_err(|error| format!("Invalid SVG: {error}"))?;
    let size = tree.size();
    let (width, height) = (size.width(), size.height());
    let scale = GLYPH_SIZE / width.max(height);
    if !scale.is_finite() || scale <= 0.0 {
        return Err("Invalid SVG dimensions.".into());
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(TILE_SIZE, TILE_SIZE)
        .ok_or_else(|| "Could not allocate the SVG preview.".to_string())?;
    let x = (TILE_SIZE as f32 - width * scale) / 2.0;
    let y = (TILE_SIZE as f32 - height * scale) / 2.0;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, x, y),
        &mut pixmap.as_mut(),
    );
    if !pixmap.data().chunks_exact(4).any(|pixel| pixel[3] != 0) {
        return Err("The SVG does not contain a visible icon.".into());
    }
    pixmap.encode_png().map_err(|error| error.to_string())
}

#[cfg(test)]
mod custom_project_icon_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn svg_rasterization_centers_and_bounds_a_static_icon() {
        let png = rasterize_project_svg(
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 20">
                <rect width="40" height="20" fill="red"/>
            </svg>"#,
        )
        .unwrap();
        let icon = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(icon.dimensions(), (256, 256));
        assert_eq!(icon.get_pixel(128, 128).0, [255, 0, 0, 255]);
        assert_eq!(icon.get_pixel(0, 0).0, [0, 0, 0, 0]);
    }

    #[test]
    fn svg_current_color_defaults_to_white_and_respects_explicit_color() {
        for (color, expected) in [
            ("", [255, 255, 255, 255]),
            (" color=\"red\"", [255, 0, 0, 255]),
        ] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"{color}>
                    <rect width="20" height="20" fill="currentColor"/>
                </svg>"#
            );
            let png = rasterize_project_svg(svg.as_bytes()).unwrap();
            let icon = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
                .unwrap()
                .to_rgba8();
            assert_eq!(icon.get_pixel(128, 128).0, expected);
        }
    }

    #[test]
    fn svg_rasterization_refuses_active_external_and_oversize_content() {
        for input in [
            r#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="file:///example.png"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#,
            r#"<!DOCTYPE svg [<!ENTITY x "x">]><svg xmlns="http://www.w3.org/2000/svg"/>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"/>"#,
        ] {
            assert!(rasterize_project_svg(input.as_bytes()).is_err());
        }
        assert!(rasterize_project_svg(&vec![0; 256 * 1024 + 1]).is_err());
    }

    #[test]
    fn validates_copies_replaces_and_resets_project_image() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("codux-icon-test-{}-{nonce}", std::process::id()));
        struct TempDir(PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _temp = TempDir(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.png");
        let id = "../project";
        std::fs::write(&source, b"not a PNG").unwrap();
        assert!(update_custom_project_icon(&dir, id, Some(&source)).is_err());
        assert!(custom_project_icon_path(&dir, id).is_none());

        image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]))
            .save(&source)
            .unwrap();
        update_custom_project_icon(&dir, id, Some(&source)).unwrap();
        let first = custom_project_icon_path(&dir, id).unwrap();
        assert_ne!(first, source);
        assert!(first.starts_with(dir.join("project-icons")));

        image::RgbImage::from_pixel(1, 1, image::Rgb([0, 0, 255]))
            .save(&source)
            .unwrap();
        update_custom_project_icon(&dir, id, Some(&source)).unwrap();
        let second = custom_project_icon_path(&dir, id).unwrap();
        assert_ne!(first, second);
        std::fs::remove_file(&source).unwrap();
        assert!(second.is_file());
        assert!(custom_project_icon_path(&dir, "another project").is_none());
        update_custom_project_icon(&dir, id, None).unwrap();
        assert!(custom_project_icon_path(&dir, id).is_none());
    }
}
