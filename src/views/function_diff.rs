use std::default::Default;

use cwdemangle::demangle;
use egui::{text::LayoutJob, Color32, Label, Sense};
use egui_extras::{Size, StripBuilder, TableBuilder};
use ppc750cl::Argument;
use time::format_description;

use crate::{
    app::{View, ViewState},
    jobs::Job,
    obj::{
        ObjInfo, ObjIns, ObjInsArg, ObjInsArgDiff, ObjInsDiff, ObjInsDiffKind, ObjReloc,
        ObjRelocKind, ObjSymbol,
    },
    views::{symbol_diff::match_color_for_symbol, write_text, COLOR_RED, FONT_SIZE},
};

fn write_reloc_name(reloc: &ObjReloc, color: Color32, job: &mut LayoutJob) {
    let name = reloc.target.demangled_name.as_ref().unwrap_or(&reloc.target.name);
    write_text(name, Color32::LIGHT_GRAY, job);
    if reloc.target.addend != 0 {
        write_text(&format!("+{:X}", reloc.target.addend), color, job);
    }
}

fn write_reloc(reloc: &ObjReloc, color: Color32, job: &mut LayoutJob) {
    match reloc.kind {
        ObjRelocKind::PpcAddr16Lo => {
            write_reloc_name(reloc, color, job);
            write_text("@l", color, job);
        }
        ObjRelocKind::PpcAddr16Hi => {
            write_reloc_name(reloc, color, job);
            write_text("@h", color, job);
        }
        ObjRelocKind::PpcAddr16Ha => {
            write_reloc_name(reloc, color, job);
            write_text("@ha", color, job);
        }
        ObjRelocKind::PpcEmbSda21 => {
            write_reloc_name(reloc, color, job);
            write_text("@sda21", color, job);
        }
        ObjRelocKind::MipsHi16 => {
            write_text("%hi(", color, job);
            write_reloc_name(reloc, color, job);
            write_text(")", color, job);
        }
        ObjRelocKind::MipsLo16 => {
            write_text("%lo(", color, job);
            write_reloc_name(reloc, color, job);
            write_text(")", color, job);
        }
        ObjRelocKind::Absolute
        | ObjRelocKind::PpcRel24
        | ObjRelocKind::PpcRel14
        | ObjRelocKind::Mips32
        | ObjRelocKind::Mips26 => {
            write_reloc_name(reloc, color, job);
        }
    };
}

fn write_ins(
    ins: &ObjIns,
    diff_kind: &ObjInsDiffKind,
    args: &[Option<ObjInsArgDiff>],
    base_addr: u32,
    job: &mut LayoutJob,
) {
    let base_color = match diff_kind {
        ObjInsDiffKind::None | ObjInsDiffKind::OpMismatch | ObjInsDiffKind::ArgMismatch => {
            Color32::GRAY
        }
        ObjInsDiffKind::Replace => Color32::LIGHT_BLUE,
        ObjInsDiffKind::Delete => COLOR_RED,
        ObjInsDiffKind::Insert => Color32::GREEN,
    };
    write_text(
        &format!("{:<11}", ins.mnemonic),
        match diff_kind {
            ObjInsDiffKind::OpMismatch => Color32::LIGHT_BLUE,
            _ => base_color,
        },
        job,
    );
    let mut writing_offset = false;
    for (i, arg) in ins.args.iter().enumerate() {
        if i == 0 {
            write_text(" ", base_color, job);
        }
        if i > 0 && !writing_offset {
            write_text(", ", base_color, job);
        }
        let color = if let Some(diff) = args.get(i).and_then(|a| a.as_ref()) {
            COLOR_ROTATION[diff.idx % COLOR_ROTATION.len()]
        } else {
            base_color
        };
        match arg {
            ObjInsArg::PpcArg(arg) => match arg {
                Argument::Offset(val) => {
                    write_text(&format!("{}", val), color, job);
                    write_text("(", base_color, job);
                    writing_offset = true;
                    continue;
                }
                Argument::Uimm(_) | Argument::Simm(_) => {
                    write_text(&format!("{}", arg), color, job);
                }
                _ => {
                    write_text(&format!("{}", arg), color, job);
                }
            },
            ObjInsArg::Reloc => {
                write_reloc(ins.reloc.as_ref().unwrap(), base_color, job);
            }
            ObjInsArg::RelocWithBase => {
                write_reloc(ins.reloc.as_ref().unwrap(), base_color, job);
                write_text("(", base_color, job);
                writing_offset = true;
                continue;
            }
            ObjInsArg::MipsArg(str) => {
                write_text(str.strip_prefix('$').unwrap_or(str), color, job);
            }
            ObjInsArg::BranchOffset(offset) => {
                let addr = offset + ins.address as i32 - base_addr as i32;
                write_text(&format!("{:x}", addr), color, job);
            }
        }
        if writing_offset {
            write_text(")", base_color, job);
            writing_offset = false;
        }
    }
}

fn ins_hover_ui(ui: &mut egui::Ui, ins: &ObjIns) {
    ui.scope(|ui| {
        ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
        ui.style_mut().wrap = Some(false);

        ui.label(format!("{:02X?}", ins.code.to_be_bytes()));

        for arg in &ins.args {
            if let ObjInsArg::PpcArg(arg) = arg {
                match arg {
                    Argument::Uimm(v) => {
                        ui.label(format!("{} == {}", v, v.0));
                    }
                    Argument::Simm(v) => {
                        ui.label(format!("{} == {}", v, v.0));
                    }
                    Argument::Offset(v) => {
                        ui.label(format!("{} == {}", v, v.0));
                    }
                    _ => {}
                }
            }
        }

        if let Some(reloc) = &ins.reloc {
            ui.label(format!("Relocation type: {:?}", reloc.kind));
            ui.colored_label(Color32::WHITE, format!("Name: {}", reloc.target.name));
            if let Some(section) = &reloc.target_section {
                ui.colored_label(Color32::WHITE, format!("Section: {}", section));
                ui.colored_label(Color32::WHITE, format!("Address: {:x}", reloc.target.address));
                ui.colored_label(Color32::WHITE, format!("Size: {:x}", reloc.target.size));
            } else {
                ui.colored_label(Color32::WHITE, "Extern".to_string());
            }
        }
    });
}

fn ins_context_menu(ui: &mut egui::Ui, ins: &ObjIns) {
    ui.scope(|ui| {
        ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
        ui.style_mut().wrap = Some(false);

        // if ui.button("Copy hex").clicked() {}

        for arg in &ins.args {
            if let ObjInsArg::PpcArg(arg) = arg {
                match arg {
                    Argument::Uimm(v) => {
                        if ui.button(format!("Copy \"{}\"", v)).clicked() {
                            ui.output().copied_text = format!("{}", v);
                            ui.close_menu();
                        }
                        if ui.button(format!("Copy \"{}\"", v.0)).clicked() {
                            ui.output().copied_text = format!("{}", v.0);
                            ui.close_menu();
                        }
                    }
                    Argument::Simm(v) => {
                        if ui.button(format!("Copy \"{}\"", v)).clicked() {
                            ui.output().copied_text = format!("{}", v);
                            ui.close_menu();
                        }
                        if ui.button(format!("Copy \"{}\"", v.0)).clicked() {
                            ui.output().copied_text = format!("{}", v.0);
                            ui.close_menu();
                        }
                    }
                    Argument::Offset(v) => {
                        if ui.button(format!("Copy \"{}\"", v)).clicked() {
                            ui.output().copied_text = format!("{}", v);
                            ui.close_menu();
                        }
                        if ui.button(format!("Copy \"{}\"", v.0)).clicked() {
                            ui.output().copied_text = format!("{}", v.0);
                            ui.close_menu();
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(reloc) = &ins.reloc {
            if let Some(name) = &reloc.target.demangled_name {
                if ui.button(format!("Copy \"{}\"", name)).clicked() {
                    ui.output().copied_text = name.clone();
                    ui.close_menu();
                }
            }
            if ui.button(format!("Copy \"{}\"", reloc.target.name)).clicked() {
                ui.output().copied_text = reloc.target.name.clone();
                ui.close_menu();
            }
        }
    });
}

const COLOR_ROTATION: [Color32; 9] = [
    Color32::from_rgb(255, 0, 255),
    Color32::from_rgb(0, 255, 255),
    Color32::from_rgb(0, 128, 0),
    Color32::from_rgb(255, 0, 0),
    Color32::from_rgb(255, 255, 0),
    Color32::from_rgb(255, 192, 203),
    Color32::from_rgb(0, 0, 255),
    Color32::from_rgb(0, 255, 0),
    Color32::from_rgb(128, 128, 128),
];

fn find_symbol<'a>(obj: &'a ObjInfo, section_name: &str, name: &str) -> Option<&'a ObjSymbol> {
    let section = obj.sections.iter().find(|s| s.name == section_name)?;
    section.symbols.iter().find(|s| s.name == name)
}

fn asm_row_ui(ui: &mut egui::Ui, ins_diff: &ObjInsDiff, symbol: &ObjSymbol) {
    if ins_diff.kind != ObjInsDiffKind::None {
        ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, ui.visuals().faint_bg_color);
    }
    let mut job = LayoutJob::default();
    if let Some(ins) = &ins_diff.ins {
        let base_color = match ins_diff.kind {
            ObjInsDiffKind::None | ObjInsDiffKind::OpMismatch | ObjInsDiffKind::ArgMismatch => {
                Color32::GRAY
            }
            ObjInsDiffKind::Replace => Color32::LIGHT_BLUE,
            ObjInsDiffKind::Delete => COLOR_RED,
            ObjInsDiffKind::Insert => Color32::GREEN,
        };
        write_text(
            &format!("{:<6}", format!("{:x}:", ins.address - symbol.address as u32)),
            base_color,
            &mut job,
        );
        if let Some(branch) = &ins_diff.branch_from {
            write_text("~> ", COLOR_ROTATION[branch.branch_idx % COLOR_ROTATION.len()], &mut job);
        } else {
            write_text("   ", base_color, &mut job);
        }
        write_ins(ins, &ins_diff.kind, &ins_diff.arg_diff, symbol.address as u32, &mut job);
        if let Some(branch) = &ins_diff.branch_to {
            write_text(" ~>", COLOR_ROTATION[branch.branch_idx % COLOR_ROTATION.len()], &mut job);
        }
        ui.add(Label::new(job).sense(Sense::click()))
            .on_hover_ui_at_pointer(|ui| ins_hover_ui(ui, ins))
            .context_menu(|ui| ins_context_menu(ui, ins));
    } else {
        ui.label("");
    }
}

fn asm_table_ui(
    table: TableBuilder<'_>,
    left_obj: &ObjInfo,
    right_obj: &ObjInfo,
    fn_name: &str,
) -> Option<()> {
    let left_symbol = find_symbol(left_obj, ".text", fn_name)?;
    let right_symbol = find_symbol(right_obj, ".text", fn_name)?;
    table.body(|body| {
        body.rows(FONT_SIZE, left_symbol.instructions.len(), |row_index, mut row| {
            row.col(|ui| {
                asm_row_ui(ui, &left_symbol.instructions[row_index], left_symbol);
            });
            row.col(|ui| {
                asm_row_ui(ui, &right_symbol.instructions[row_index], right_symbol);
            });
        });
    });
    Some(())
}

pub fn function_diff_ui(ui: &mut egui::Ui, view_state: &mut ViewState) -> bool {
    let mut rebuild = false;
    if let (Some(result), Some(selected_symbol)) = (&view_state.build, &view_state.selected_symbol)
    {
        StripBuilder::new(ui)
            .size(Size::exact(20.0))
            .size(Size::exact(40.0))
            .size(Size::remainder())
            .vertical(|mut strip| {
                strip.strip(|builder| {
                    builder.sizes(Size::remainder(), 2).horizontal(|mut strip| {
                        strip.cell(|ui| {
                            ui.horizontal(|ui| {
                                if ui.button("Back").clicked() {
                                    view_state.current_view = View::SymbolDiff;
                                }
                            });
                        });
                        strip.cell(|ui| {
                            ui.horizontal(|ui| {
                                if ui.button("Build").clicked() {
                                    rebuild = true;
                                }
                                ui.scope(|ui| {
                                    ui.style_mut().override_text_style =
                                        Some(egui::TextStyle::Monospace);
                                    ui.style_mut().wrap = Some(false);
                                    if view_state
                                        .jobs
                                        .iter()
                                        .any(|job| job.job_type == Job::ObjDiff)
                                    {
                                        ui.label("Building...");
                                    } else {
                                        ui.label("Last built:");
                                        let format =
                                            format_description::parse("[hour]:[minute]:[second]")
                                                .unwrap();
                                        ui.label(
                                            result
                                                .time
                                                .to_offset(view_state.utc_offset)
                                                .format(&format)
                                                .unwrap(),
                                        );
                                    }
                                });
                            });
                        });
                    });
                });
                strip.strip(|builder| {
                    builder.sizes(Size::remainder(), 2).horizontal(|mut strip| {
                        let demangled = demangle(selected_symbol);
                        strip.cell(|ui| {
                            ui.scope(|ui| {
                                ui.style_mut().override_text_style =
                                    Some(egui::TextStyle::Monospace);
                                ui.style_mut().wrap = Some(false);
                                ui.colored_label(
                                    Color32::WHITE,
                                    demangled.as_ref().unwrap_or(selected_symbol),
                                );
                                ui.label("Diff target:");
                                ui.separator();
                            });
                        });
                        strip.cell(|ui| {
                            ui.scope(|ui| {
                                ui.style_mut().override_text_style =
                                    Some(egui::TextStyle::Monospace);
                                ui.style_mut().wrap = Some(false);
                                if let Some(obj) = &result.second_obj {
                                    if let Some(symbol) = find_symbol(obj, ".text", selected_symbol)
                                    {
                                        ui.colored_label(
                                            match_color_for_symbol(symbol),
                                            &format!("{:.0}%", symbol.match_percent),
                                        );
                                    }
                                }
                                ui.label("Diff base:");
                                ui.separator();
                            });
                        });
                    });
                });
                strip.cell(|ui| {
                    if let (Some(left_obj), Some(right_obj)) =
                        (&result.first_obj, &result.second_obj)
                    {
                        let table = TableBuilder::new(ui)
                            .striped(false)
                            .cell_layout(egui::Layout::left_to_right(egui::Align::Min))
                            .column(Size::relative(0.5))
                            .column(Size::relative(0.5))
                            .resizable(false);
                        asm_table_ui(table, left_obj, right_obj, selected_symbol);
                    }
                });
            });
    }
    rebuild
}
