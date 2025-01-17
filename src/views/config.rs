#[cfg(windows)]
use std::string::FromUtf16Error;
use std::sync::{Arc, RwLock};

#[cfg(windows)]
use anyhow::{Context, Result};

use crate::{
    app::{AppConfig, DiffKind, ViewState},
    jobs::{bindiff::queue_bindiff, objdiff::queue_build},
};

#[cfg(windows)]
fn process_utf16(bytes: &[u8]) -> Result<String, FromUtf16Error> {
    let u16_bytes: Vec<u16> = bytes
        .chunks_exact(2)
        .filter_map(|c| Some(u16::from_ne_bytes(c.try_into().ok()?)))
        .collect();
    String::from_utf16(&u16_bytes)
}

#[cfg(windows)]
fn wsl_cmd(args: &[&str]) -> Result<String> {
    use std::{os::windows::process::CommandExt, process::Command};
    let output = Command::new("wsl")
        .args(args)
        .creation_flags(winapi::um::winbase::CREATE_NO_WINDOW)
        .output()
        .context("Failed to execute wsl")?;
    process_utf16(&output.stdout).context("Failed to process stdout")
}

#[cfg(windows)]
fn fetch_wsl2_distros() -> Vec<String> {
    wsl_cmd(&["-l", "-q"])
        .map(|stdout| {
            stdout
                .split('\n')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}

pub fn config_ui(ui: &mut egui::Ui, config: &Arc<RwLock<AppConfig>>, view_state: &mut ViewState) {
    let mut config_guard = config.write().unwrap();
    let AppConfig {
        custom_make,
        available_wsl_distros,
        selected_wsl_distro,
        project_dir,
        target_obj_dir,
        base_obj_dir,
        obj_path,
        build_target,
        left_obj,
        right_obj,
        project_dir_change,
    } = &mut *config_guard;

    ui.heading("Build config");

    #[cfg(windows)]
    {
        if available_wsl_distros.is_none() {
            *available_wsl_distros = Some(fetch_wsl2_distros());
        }
        egui::ComboBox::from_label("Run in WSL2")
            .selected_text(selected_wsl_distro.as_ref().unwrap_or(&"None".to_string()))
            .show_ui(ui, |ui| {
                ui.selectable_value(selected_wsl_distro, None, "None");
                for distro in available_wsl_distros.as_ref().unwrap() {
                    ui.selectable_value(selected_wsl_distro, Some(distro.clone()), distro);
                }
            });
    }
    #[cfg(not(windows))]
    {
        let _ = available_wsl_distros;
        let _ = selected_wsl_distro;
    }

    ui.label("Custom make program:");
    ui.text_edit_singleline(custom_make);

    ui.separator();

    ui.heading("Project config");

    if view_state.diff_kind == DiffKind::SplitObj {
        if ui.button("Select project dir").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                *project_dir = Some(path);
                *project_dir_change = true;
                *target_obj_dir = None;
                *base_obj_dir = None;
                *obj_path = None;
            }
        }
        if let Some(dir) = project_dir {
            ui.label(dir.to_string_lossy());
        }

        ui.separator();

        if let Some(project_dir) = project_dir {
            if ui.button("Select target build dir").clicked() {
                if let Some(path) = rfd::FileDialog::new().set_directory(&project_dir).pick_folder()
                {
                    *target_obj_dir = Some(path);
                    *obj_path = None;
                }
            }
            if let Some(dir) = target_obj_dir {
                ui.label(dir.to_string_lossy());
            }
            ui.checkbox(build_target, "Build target");

            ui.separator();

            if ui.button("Select base build dir").clicked() {
                if let Some(path) = rfd::FileDialog::new().set_directory(&project_dir).pick_folder()
                {
                    *base_obj_dir = Some(path);
                    *obj_path = None;
                }
            }
            if let Some(dir) = base_obj_dir {
                ui.label(dir.to_string_lossy());
            }

            ui.separator();
        }

        if let Some(base_dir) = base_obj_dir {
            if ui.button("Select obj").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&base_dir)
                    .add_filter("Object file", &["o", "elf"])
                    .pick_file()
                {
                    let mut new_build_obj: Option<String> = None;
                    if let Ok(obj_path) = path.strip_prefix(&base_dir) {
                        new_build_obj = Some(obj_path.display().to_string());
                    } else if let Some(build_asm_dir) = target_obj_dir {
                        if let Ok(obj_path) = path.strip_prefix(&build_asm_dir) {
                            new_build_obj = Some(obj_path.display().to_string());
                        }
                    }
                    if let Some(new_build_obj) = new_build_obj {
                        *obj_path = Some(new_build_obj);
                        view_state
                            .jobs
                            .push(queue_build(config.clone(), view_state.diff_config.clone()));
                    }
                }
            }
            if let Some(obj) = obj_path {
                ui.label(&*obj);
                if ui.button("Build").clicked() {
                    view_state
                        .jobs
                        .push(queue_build(config.clone(), view_state.diff_config.clone()));
                }
            }

            ui.separator();
        }
    } else if view_state.diff_kind == DiffKind::WholeBinary {
        if ui.button("Select left obj").clicked() {
            if let Some(path) =
                rfd::FileDialog::new().add_filter("Object file", &["o", "elf"]).pick_file()
            {
                *left_obj = Some(path);
            }
        }
        if let Some(obj) = left_obj {
            ui.label(obj.to_string_lossy());
        }

        if ui.button("Select right obj").clicked() {
            if let Some(path) =
                rfd::FileDialog::new().add_filter("Object file", &["o", "elf"]).pick_file()
            {
                *right_obj = Some(path);
            }
        }
        if let Some(obj) = right_obj {
            ui.label(obj.to_string_lossy());
        }

        if let (Some(_), Some(_)) = (left_obj, right_obj) {
            if ui.button("Build").clicked() {
                view_state.jobs.push(queue_bindiff(config.clone()));
            }
        }
    }

    ui.checkbox(&mut view_state.reverse_fn_order, "Reverse function order (deferred)");
    ui.separator();
}
