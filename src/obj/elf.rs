use std::{fs, path::Path};

use anyhow::{Context, Result};
use cwdemangle::demangle;
use flagset::Flags;
use object::{
    Architecture, File, Object, ObjectSection, ObjectSymbol, RelocationKind, RelocationTarget,
    SectionKind, Symbol, SymbolKind, SymbolSection,
};

use crate::obj::{
    ObjArchitecture, ObjInfo, ObjReloc, ObjRelocKind, ObjSection, ObjSectionKind, ObjSymbol,
    ObjSymbolFlagSet, ObjSymbolFlags,
};

fn to_obj_section_kind(kind: SectionKind) -> ObjSectionKind {
    match kind {
        SectionKind::Text => ObjSectionKind::Code,
        SectionKind::Data | SectionKind::ReadOnlyData => ObjSectionKind::Data,
        SectionKind::UninitializedData => ObjSectionKind::Bss,
        _ => panic!("Unhandled section kind {:?}", kind),
    }
}

fn to_obj_symbol(obj_file: &File<'_>, symbol: &Symbol<'_, '_>, addend: i64) -> Result<ObjSymbol> {
    let mut name = symbol.name().context("Failed to process symbol name")?;
    if name.is_empty() {
        println!("Found empty sym: {:?}", symbol);
        name = "?";
    }
    let mut flags = ObjSymbolFlagSet(ObjSymbolFlags::none());
    if symbol.is_global() {
        flags = ObjSymbolFlagSet(flags.0 | ObjSymbolFlags::Global);
    }
    if symbol.is_local() {
        flags = ObjSymbolFlagSet(flags.0 | ObjSymbolFlags::Local);
    }
    if symbol.is_common() {
        flags = ObjSymbolFlagSet(flags.0 | ObjSymbolFlags::Common);
    }
    if symbol.is_weak() {
        flags = ObjSymbolFlagSet(flags.0 | ObjSymbolFlags::Weak);
    }
    let section_address = if let Some(section) =
        symbol.section_index().and_then(|idx| obj_file.section_by_index(idx).ok())
    {
        symbol.address() - section.address()
    } else {
        symbol.address()
    };
    Ok(ObjSymbol {
        name: name.to_string(),
        demangled_name: demangle(name),
        address: symbol.address(),
        section_address,
        size: symbol.size(),
        size_known: symbol.size() != 0,
        flags,
        addend,
        diff_symbol: None,
        instructions: vec![],
        match_percent: 0.0,
    })
}

const R_PPC_ADDR16_LO: u32 = 4;
const R_PPC_ADDR16_HI: u32 = 5;
const R_PPC_ADDR16_HA: u32 = 6;
const R_PPC_REL24: u32 = 10;
const R_PPC_REL14: u32 = 11;
const R_PPC_EMB_SDA21: u32 = 109;

const R_MIPS_26: u32 = 4;
const R_MIPS_HI16: u32 = 5;
const R_MIPS_LO16: u32 = 6;

fn filter_sections(obj_file: &File<'_>) -> Result<Vec<ObjSection>> {
    let mut result = Vec::<ObjSection>::new();
    for section in obj_file.sections() {
        if section.size() == 0 {
            continue;
        }
        if section.kind() != SectionKind::Text
            && section.kind() != SectionKind::Data
            && section.kind() != SectionKind::ReadOnlyData
            && section.kind() != SectionKind::UninitializedData
        {
            continue;
        }
        let name = section.name().context("Failed to process section name")?;
        let data = section.data().context("Failed to read section data")?;
        result.push(ObjSection {
            name: name.to_string(),
            kind: to_obj_section_kind(section.kind()),
            address: section.address(),
            size: section.size(),
            data: data.to_vec(),
            index: section.index().0,
            symbols: Vec::new(),
            relocations: Vec::new(),
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

fn symbols_by_section(obj_file: &File<'_>, section: &ObjSection) -> Result<Vec<ObjSymbol>> {
    let mut result = Vec::<ObjSymbol>::new();
    for symbol in obj_file.symbols() {
        if symbol.kind() == SymbolKind::Section {
            continue;
        }
        if let Some(index) = symbol.section().index() {
            if index.0 == section.index {
                if symbol.is_local() && section.kind == ObjSectionKind::Code {
                    // TODO strip local syms in diff?
                    let name = symbol.name().context("Failed to process symbol name")?;
                    if symbol.size() == 0 || name.starts_with("lbl_") {
                        continue;
                    }
                }
                result.push(to_obj_symbol(obj_file, &symbol, 0)?);
            }
        }
    }
    result.sort_by_key(|v| v.address);
    let mut iter = result.iter_mut().peekable();
    while let Some(symbol) = iter.next() {
        if symbol.size == 0 {
            if let Some(next_symbol) = iter.peek() {
                symbol.size = next_symbol.address - symbol.address;
            } else {
                symbol.size = (section.address + section.size) - symbol.address;
            }
        }
    }
    Ok(result)
}

fn common_symbols(obj_file: &File<'_>) -> Result<Vec<ObjSymbol>> {
    let mut result = Vec::<ObjSymbol>::new();
    for symbol in obj_file.symbols() {
        if symbol.is_common() {
            result.push(to_obj_symbol(obj_file, &symbol, 0)?);
        }
    }
    Ok(result)
}

fn find_section_symbol(
    obj_file: &File<'_>,
    target: &Symbol<'_, '_>,
    address: u64,
) -> Result<ObjSymbol> {
    let section_index =
        target.section_index().ok_or_else(|| anyhow::Error::msg("Unknown section index"))?;
    let section = obj_file.section_by_index(section_index)?;
    let mut closest_symbol: Option<Symbol<'_, '_>> = None;
    for symbol in obj_file.symbols() {
        if !matches!(symbol.section_index(), Some(idx) if idx == section_index) {
            continue;
        }
        if symbol.kind() == SymbolKind::Section || symbol.address() != address {
            if symbol.address() < address
                && symbol.size() != 0
                && (closest_symbol.is_none()
                    || matches!(&closest_symbol, Some(s) if s.address() <= symbol.address()))
            {
                closest_symbol = Some(symbol);
            }
            continue;
        }
        return to_obj_symbol(obj_file, &symbol, 0);
    }
    let (name, offset) = closest_symbol
        .and_then(|s| s.name().map(|n| (n, s.address())).ok())
        .or_else(|| section.name().map(|n| (n, section.address())).ok())
        .unwrap_or(("<unknown>", 0));
    let offset_addr = address - offset;
    Ok(ObjSymbol {
        name: name.to_string(),
        demangled_name: None,
        address,
        section_address: address - section.address(),
        size: 0,
        size_known: false,
        flags: Default::default(),
        addend: offset_addr as i64,
        diff_symbol: None,
        instructions: vec![],
        match_percent: 0.0,
    })
}

fn relocations_by_section(
    arch: ObjArchitecture,
    obj_file: &File<'_>,
    section: &mut ObjSection,
) -> Result<Vec<ObjReloc>> {
    let obj_section = obj_file
        .section_by_name(&section.name)
        .ok_or_else(|| anyhow::Error::msg("Failed to locate section"))?;
    let mut relocations = Vec::<ObjReloc>::new();
    for (address, reloc) in obj_section.relocations() {
        let symbol = match reloc.target() {
            RelocationTarget::Symbol(idx) => obj_file
                .symbol_by_index(idx)
                .context("Failed to locate relocation target symbol")?,
            _ => {
                return Err(anyhow::Error::msg(format!(
                    "Unhandled relocation target: {:?}",
                    reloc.target()
                )));
            }
        };
        let kind = match reloc.kind() {
            RelocationKind::Absolute => ObjRelocKind::Absolute,
            RelocationKind::Elf(kind) => match arch {
                ObjArchitecture::PowerPc => match kind {
                    R_PPC_ADDR16_LO => ObjRelocKind::PpcAddr16Lo,
                    R_PPC_ADDR16_HI => ObjRelocKind::PpcAddr16Hi,
                    R_PPC_ADDR16_HA => ObjRelocKind::PpcAddr16Ha,
                    R_PPC_REL24 => ObjRelocKind::PpcRel24,
                    R_PPC_REL14 => ObjRelocKind::PpcRel14,
                    R_PPC_EMB_SDA21 => ObjRelocKind::PpcEmbSda21,
                    _ => {
                        return Err(anyhow::Error::msg(format!(
                            "Unhandled PPC relocation type: {}",
                            kind
                        )))
                    }
                },
                ObjArchitecture::Mips => match kind {
                    R_MIPS_26 => ObjRelocKind::Mips26,
                    R_MIPS_HI16 => ObjRelocKind::MipsHi16,
                    R_MIPS_LO16 => ObjRelocKind::MipsLo16,
                    _ => {
                        return Err(anyhow::Error::msg(format!(
                            "Unhandled MIPS relocation type: {}",
                            kind
                        )))
                    }
                },
            },
            _ => {
                return Err(anyhow::Error::msg(format!(
                    "Unhandled relocation type: {:?}",
                    reloc.kind()
                )))
            }
        };
        let target_section = match symbol.section() {
            SymbolSection::Common => Some(".comm".to_string()),
            SymbolSection::Section(idx) => {
                obj_file.section_by_index(idx).and_then(|s| s.name().map(|s| s.to_string())).ok()
            }
            _ => None,
        };
        // println!("Reloc: {:?}, symbol: {:?}", reloc, symbol);
        let target = match symbol.kind() {
            SymbolKind::Text | SymbolKind::Data | SymbolKind::Unknown => {
                to_obj_symbol(obj_file, &symbol, reloc.addend())
            }
            SymbolKind::Section => {
                let addend = if reloc.has_implicit_addend() {
                    let addend = u32::from_be_bytes(
                        section.data[address as usize..address as usize + 4].try_into()?,
                    );
                    match kind {
                        ObjRelocKind::MipsHi16 | ObjRelocKind::MipsLo16 => {
                            (addend & 0x0000FFFF) * 4
                        }
                        ObjRelocKind::Mips26 => (addend & 0x03FFFFFF) * 4,
                        _ => todo!(),
                    }
                } else {
                    let addend = reloc.addend();
                    if addend < 0 {
                        return Err(anyhow::Error::msg(format!(
                            "Negative addend in section reloc: {}",
                            addend
                        )));
                    }
                    addend as u32
                };
                find_section_symbol(obj_file, &symbol, addend as u64)
            }
            _ => Err(anyhow::Error::msg(format!(
                "Unhandled relocation symbol type {:?}",
                symbol.kind()
            ))),
        }?;
        relocations.push(ObjReloc { kind, address, target, target_section });
    }
    Ok(relocations)
}

pub fn read(obj_path: &Path) -> Result<ObjInfo> {
    let bin_data = fs::read(obj_path)?;
    let obj_file = File::parse(&*bin_data)?;
    let architecture = match obj_file.architecture() {
        Architecture::PowerPc => ObjArchitecture::PowerPc,
        Architecture::Mips => ObjArchitecture::Mips,
        _ => {
            return Err(anyhow::Error::msg(format!(
                "Unsupported architecture: {:?}",
                obj_file.architecture()
            )))
        }
    };
    let mut result = ObjInfo {
        architecture,
        path: obj_path.to_owned(),
        sections: filter_sections(&obj_file)?,
        common: common_symbols(&obj_file)?,
    };
    for section in &mut result.sections {
        section.symbols = symbols_by_section(&obj_file, section)?;
        section.relocations = relocations_by_section(architecture, &obj_file, section)?;
    }
    Ok(result)
}
