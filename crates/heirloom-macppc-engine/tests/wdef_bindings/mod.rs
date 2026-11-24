//! TEST-ONLY reader of binding files (`bindings/macppc/*.sexp`, the one s-expression grammar: lists, symbols,
//! "strings", decimal integers, `;` comments). The product's host reads the same files with CoralOS's s-expression reader and hands
//! the engine the same values (`MacPpc::add_window_def_binding`, `Mac::add_plane_chrome_binding`); this exists so the harness and
//! tests load the DATA, not a copy of it.
#![allow(dead_code)]
use heirloom_bind::{
    FooterKeyRule, MiniToggleRule, PaletteRule, PaletteRuleKind, PixelMaskGuard, PlaneChromeBinding,
    PlaneChromeIdentity, TabProbeRule, ToolCellRule, WindowDefBinding, WindowDefIdentity, WindowMeaning, WindowRange,
};
use heirloom_ui_abi::{GuestPoint, GuestRect, IconRole, PenRole, WindowRole};

#[derive(Debug, Clone)]
pub enum S { List(Vec<S>), Atom(String), Str(String) }

fn parse(src: &str) -> Result<Vec<S>, String> {
    let mut stack: Vec<Vec<S>> = vec![vec![]];
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            ';' => { while let Some(&d) = it.peek() { if d == '\n' { break; } it.next(); } }
            '(' => stack.push(vec![]),
            ')' => { let l = stack.pop().ok_or("unbalanced )")?; stack.last_mut().ok_or("unbalanced )")?.push(S::List(l)); }
            '"' => { let mut t = String::new(); loop { match it.next() { Some('"') => break, Some(d) => t.push(d), None => return Err("unterminated string".into()) } } stack.last_mut().unwrap().push(S::Str(t)); }
            c if c.is_whitespace() => {}
            c => { let mut t = String::from(c); while let Some(&d) = it.peek() { if d.is_whitespace() || d == '(' || d == ')' { break; } t.push(d); it.next(); } stack.last_mut().unwrap().push(S::Atom(t)); }
        }
    }
    if stack.len() != 1 { return Err("unbalanced (".into()); }
    Ok(stack.pop().unwrap())
}

fn field<'a>(l: &'a [S], name: &str) -> Result<&'a [S], String> {
    l.iter().find_map(|s| match s { S::List(v) if matches!(v.first(), Some(S::Atom(a)) if a == name) => Some(&v[1..]), _ => None }).ok_or_else(|| format!("no ({name} ...)"))
}
fn int(l: &[S], name: &str) -> Result<u64, String> {
    match field(l, name)?.first() {
        Some(S::Atom(a)) => a.parse().map_err(|_| format!("({name} {a}): not a decimal integer")),
        Some(S::Str(t)) if t.starts_with("0x") => u64::from_str_radix(&t[2..], 16).map_err(|_| format!("({name} \"{t}\"): not hex")),
        x => Err(format!("({name} {x:?}): not an integer")),
    }
}
fn boolean(l: &[S], name: &str) -> Result<bool, String> {
    match field(l, name)?.first() { Some(S::Atom(a)) if a == "true" => Ok(true), Some(S::Atom(a)) if a == "false" => Ok(false), x => Err(format!("({name} {x:?}): not a boolean")) }
}

/// Every `(window-definition ...)` in `src`; a malformed one is an Err naming what is wrong.
pub fn read(src: &str) -> Result<Vec<WindowDefBinding>, String> {
    let mut out = vec![];
    for top in parse(src)? {
        let S::List(v) = top else { return Err("a top-level atom".into()) };
        if !matches!(v.first(), Some(S::Atom(a)) if a == "window-definition") { continue; }
        let id = match field(&v, "id")?.first() { Some(S::Str(t)) => t.clone(), x => return Err(format!("(id {x:?})")) };
        let fc = field(&v, "fragment-code")?;
        let identity = WindowDefIdentity::FragmentCode { code_length: int(fc, "length")? as u32, code_fnv1a64: int(fc, "fnv1a64")?, entry_offset: int(fc, "entry-offset")? as u32 };
        let m = field(&v, "meaning")?;
        let role = match field(m, "role")?.first() { Some(S::Atom(a)) if a == "dialog" => WindowRole::Dialog, Some(S::Atom(a)) if a == "document" => WindowRole::Document, Some(S::Atom(a)) if a == "utility" => WindowRole::Utility, x => return Err(format!("(role {x:?})")) };
        out.push(WindowDefBinding { id, identity, meaning: WindowMeaning { role, titled: boolean(m, "titled")?, movable: boolean(m, "movable")?, closable: boolean(m, "closable")?, resizable: boolean(m, "resizable")? } });
    }
    Ok(out)
}

/// The compat repo's own window-definition binding files for `platform`.
pub fn load(platform: &str) -> Vec<WindowDefBinding> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bindings").join(platform);
    let mut out = vec![];
    for e in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "sexp") { out.extend(read(&std::fs::read_to_string(&p).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display()))); }
    }
    out
}

fn opt_field<'a>(l: &'a [S], name: &str) -> Option<&'a [S]> {
    l.iter().find_map(|s| match s { S::List(v) if matches!(v.first(), Some(S::Atom(a)) if a == name) => Some(&v[1..]), _ => None })
}
fn i32_arg(s: Option<&S>, ctx: &str) -> Result<i32, String> {
    match s {
        Some(S::Atom(a)) => a.parse().map_err(|_| format!("{ctx}: {a} is not i32")),
        x => Err(format!("{ctx}: expected i32, got {x:?}")),
    }
}
fn u32_hex_or_dec(s: Option<&S>, ctx: &str) -> Result<u32, String> {
    match s {
        Some(S::Atom(a)) => a.parse().map_err(|_| format!("{ctx}: {a} is not u32")),
        Some(S::Str(t)) if t.starts_with("0x") || t.starts_with("0X") => u32::from_str_radix(&t[2..], 16).map_err(|_| format!("{ctx}: {t} is not hex u32")),
        x => Err(format!("{ctx}: expected u32 or 0x hex, got {x:?}")),
    }
}
fn rect4(args: &[S], ctx: &str) -> Result<GuestRect, String> {
    if args.len() != 4 { return Err(format!("{ctx}: expected 4 ints")); }
    Ok(GuestRect { left: i32_arg(args.first(), ctx)?, top: i32_arg(args.get(1), ctx)?, width: i32_arg(args.get(2), ctx)?, height: i32_arg(args.get(3), ctx)? })
}
fn pt2(args: &[S], ctx: &str) -> Result<GuestPoint, String> {
    if args.len() != 2 { return Err(format!("{ctx}: expected 2 ints")); }
    Ok(GuestPoint { x: i32_arg(args.first(), ctx)?, y: i32_arg(args.get(1), ctx)? })
}
fn icon_role(s: Option<&S>, ctx: &str) -> Result<IconRole, String> {
    let Some(S::Atom(a)) = s else { return Err(format!("{ctx}: expected icon symbol, got {s:?}")) };
    match a.as_str() {
        "disk" => Ok(IconRole::Disk), "volume" => Ok(IconRole::Volume), "drawer" => Ok(IconRole::Drawer),
        "tool" => Ok(IconRole::Tool), "project" => Ok(IconRole::Project), "trash" => Ok(IconRole::Trash),
        "device" => Ok(IconRole::Device), "kick" => Ok(IconRole::Kick),
        "align-left" => Ok(IconRole::AlignLeft), "align-center" => Ok(IconRole::AlignCenter),
        "align-right" => Ok(IconRole::AlignRight), "align-justify" => Ok(IconRole::AlignJustify),
        "tab-stop" => Ok(IconRole::TabStop), "tab-decimal" => Ok(IconRole::TabDecimal),
        "box-select" => Ok(IconRole::BoxSelect), "move" => Ok(IconRole::Move), "lasso" => Ok(IconRole::Lasso),
        "magic-wand" => Ok(IconRole::MagicWand), "crop" => Ok(IconRole::Crop), "paintbrush" => Ok(IconRole::Paintbrush),
        "stamp" => Ok(IconRole::Stamp), "eraser" => Ok(IconRole::Eraser), "paint-bucket" => Ok(IconRole::PaintBucket),
        "droplet" => Ok(IconRole::Droplet), "mouse-pointer" => Ok(IconRole::MousePointer), "type" => Ok(IconRole::Type),
        "pen-tool" => Ok(IconRole::PenTool), "shapes" => Ok(IconRole::Shapes), "pipette" => Ok(IconRole::Pipette),
        "hand" => Ok(IconRole::Hand), "zoom-in" => Ok(IconRole::ZoomIn), "zoom-out" => Ok(IconRole::ZoomOut),
        "eye" => Ok(IconRole::Eye), "link" => Ok(IconRole::Link), "lock" => Ok(IconRole::Lock),
        "file-plus" => Ok(IconRole::FilePlus), "folder-plus" => Ok(IconRole::FolderPlus), "layers" => Ok(IconRole::Layers),
        "sliders" => Ok(IconRole::Sliders), "square" => Ok(IconRole::Square), "squares-unite" => Ok(IconRole::SquaresUnite),
        "squares-subtract" => Ok(IconRole::SquaresSubtract), "squares-intersect" => Ok(IconRole::SquaresIntersect),
        "square-dashed" => Ok(IconRole::SquareDashed),
        other => Err(format!("{ctx}: unknown icon role {other:?}")),
    }
}
fn pen_role(s: Option<&S>, ctx: &str) -> Result<PenRole, String> {
    let Some(S::Atom(a)) = s else { return Err(format!("{ctx}: expected pen symbol")) };
    match a.as_str() {
        "surface" => Ok(PenRole::Surface), "background" => Ok(PenRole::Background),
        "fill" => Ok(PenRole::Fill), "highlight" => Ok(PenRole::Highlight),
        other => Err(format!("{ctx}: unknown pen role {other:?}")),
    }
}

/// Every `(plane-chrome ...)` in `src`.
pub fn read_plane_chrome(src: &str) -> Result<Vec<PlaneChromeBinding>, String> {
    let mut out = vec![];
    for top in parse(src)? {
        let S::List(v) = top else { return Err("a top-level atom".into()) };
        if !matches!(v.first(), Some(S::Atom(a)) if a == "plane-chrome") { continue; }
        let id = match field(&v, "id")?.first() { Some(S::Str(t)) => t.clone(), x => return Err(format!("(id {x:?})")) };
        let res = field(&v, "resource")?;
        let res_type = match field(res, "type")?.first() {
            Some(S::Str(s)) if s.len() == 4 => [s.as_bytes()[0], s.as_bytes()[1], s.as_bytes()[2], s.as_bytes()[3]],
            x => return Err(format!("(resource type {x:?})")),
        };
        let identity = PlaneChromeIdentity::Resource {
            res_type,
            res_id: int(res, "id")? as i16,
            length: int(res, "length")? as u32,
            fnv1a64: int(res, "fnv1a64")?,
        };
        let window_range = match opt_field(&v, "window-range") {
            None => None,
            Some(a) => {
                if a.len() != 4 { return Err("window-range expects 4 ints".into()); }
                Some(WindowRange { min_w: i32_arg(a.first(), "wr")?, max_w: i32_arg(a.get(1), "wr")?, min_h: i32_arg(a.get(2), "wr")?, max_h: i32_arg(a.get(3), "wr")? })
            }
        };
        let suppress_bottom_right = match opt_field(&v, "suppress-bottom-right") {
            None => None,
            Some(a) => Some((i32_arg(a.first(), "sbr")?, i32_arg(a.get(1), "sbr")?)),
        };
        let mut pixel_guards = vec![];
        let mut palette_rules = vec![];
        for item in &v[1..] {
            let S::List(l) = item else { continue };
            let Some(S::Atom(tag)) = l.first() else { continue };
            let args = &l[1..];
            match tag.as_str() {
                "pixel-guard" => pixel_guards.push(PixelMaskGuard {
                    rect: rect4(field(args, "rect")?, "guard rect")?,
                    dark_luma: int(args, "dark-luma")? as u8,
                    fnv1a64: int(args, "fnv1a64")?,
                }),
                "palette-surface" => palette_rules.push(PaletteRule {
                    rect: rect4(field(args, "rect")?, "ps rect")?,
                    clear_rect: opt_field(args, "clear-rect").map(|a| rect4(a, "ps clear")).transpose()?,
                    kind: PaletteRuleKind::SurfaceFill { pen_role: opt_field(args, "pen").map(|a| pen_role(a.first(), "pen")).transpose()?.unwrap_or(PenRole::Surface) },
                }),
                "palette-keep-pixels" => palette_rules.push(PaletteRule {
                    rect: rect4(field(args, "rect")?, "pkp rect")?,
                    clear_rect: opt_field(args, "clear-rect").map(|a| rect4(a, "pkp clear")).transpose()?,
                    kind: PaletteRuleKind::KeepPixels,
                }),
                "palette-tabs" => {
                    let mut tabs = vec![];
                    for sub in args {
                        if let S::List(tl) = sub {
                            if matches!(tl.first(), Some(S::Atom(a)) if a == "tab") {
                                let label = match field(&tl[1..], "label")?.first() { Some(S::Str(s)) => s.clone(), x => return Err(format!("tab label {x:?}")) };
                                let probe_rect = rect4(field(&tl[1..], "probe")?, "tab probe")?;
                                tabs.push(TabProbeRule { label, probe_rect });
                            }
                        }
                    }
                    palette_rules.push(PaletteRule {
                        rect: rect4(field(args, "rect")?, "tabs rect")?,
                        clear_rect: opt_field(args, "clear-rect").map(|a| rect4(a, "tabs clear")).transpose()?,
                        kind: PaletteRuleKind::Tabs { tabs, flyout: opt_field(args, "flyout").map(|_| boolean(args, "flyout")).transpose()?.unwrap_or(true) },
                    });
                }
                "palette-tool-grid" => {
                    let mut tools = vec![];
                    for sub in args {
                        if let S::List(tl) = sub {
                            if matches!(tl.first(), Some(S::Atom(a)) if a == "tool") {
                                let ta = &tl[1..];
                                tools.push(ToolCellRule {
                                    icon: icon_role(field(ta, "icon")?.first(), "tool icon")?,
                                    has_flyout: opt_field(ta, "flyout").map(|_| boolean(ta, "flyout")).transpose()?.unwrap_or(false),
                                    break_after: opt_field(ta, "break-after").map(|_| boolean(ta, "break-after")).transpose()?.unwrap_or(false),
                                    probe_rect: rect4(field(ta, "probe")?, "tool probe")?,
                                });
                            }
                        }
                    }
                    palette_rules.push(PaletteRule {
                        rect: rect4(field(args, "rect")?, "tg rect")?,
                        clear_rect: opt_field(args, "clear-rect").map(|a| rect4(a, "tg clear")).transpose()?,
                        kind: PaletteRuleKind::ToolGrid { cols: int(args, "cols")? as u8, tools },
                    });
                }
                "palette-color-well" => palette_rules.push(PaletteRule {
                    rect: rect4(field(args, "rect")?, "cw rect")?,
                    clear_rect: opt_field(args, "clear-rect").map(|a| rect4(a, "cw clear")).transpose()?,
                    kind: PaletteRuleKind::ColorWell {
                        fore_pt: pt2(field(args, "fore")?, "cw fore")?,
                        back_pt: pt2(field(args, "back")?, "cw back")?,
                    },
                }),
                "palette-footer" => {
                    let mut keys = vec![];
                    for sub in args {
                        if let S::List(kl) = sub {
                            if matches!(kl.first(), Some(S::Atom(a)) if a == "key") {
                                let ka = &kl[1..];
                                keys.push(FooterKeyRule {
                                    icon: icon_role(field(ka, "icon")?.first(), "footer icon")?,
                                    probe_rect: opt_field(ka, "probe").map(|a| rect4(a, "footer probe")).transpose()?,
                                });
                            }
                        }
                    }
                    palette_rules.push(PaletteRule {
                        rect: rect4(field(args, "rect")?, "footer rect")?,
                        clear_rect: opt_field(args, "clear-rect").map(|a| rect4(a, "footer clear")).transpose()?,
                        kind: PaletteRuleKind::Footer { keys },
                    });
                }
                "palette-mini-toggles" => {
                    let mut keys = vec![];
                    for sub in args {
                        if let S::List(kl) = sub {
                            if matches!(kl.first(), Some(S::Atom(a)) if a == "toggle") {
                                let ka = &kl[1..];
                                keys.push(MiniToggleRule {
                                    icon: icon_role(field(ka, "icon")?.first(), "toggle icon")?,
                                    probe_rect: rect4(field(ka, "probe")?, "toggle probe")?,
                                    threshold_luma: int(ka, "threshold-luma")? as u8,
                                });
                            }
                        }
                    }
                    palette_rules.push(PaletteRule {
                        rect: rect4(field(args, "rect")?, "mt rect")?,
                        clear_rect: opt_field(args, "clear-rect").map(|a| rect4(a, "mt clear")).transpose()?,
                        kind: PaletteRuleKind::MiniToggles { keys },
                    });
                }
                "palette-gradient-slider" => {
                    let label = match field(args, "label")?.first() { Some(S::Str(s)) => s.clone(), x => return Err(format!("gs label {x:?}")) };
                    let stops: Vec<u32> = field(args, "stops")?.iter().map(|s| u32_hex_or_dec(Some(s), "gs stop")).collect::<Result<_, _>>()?;
                    let track_rect = opt_field(args, "track").map(|a| rect4(a, "gs track")).transpose()?;
                    let default_permille = opt_field(args, "default").map(|_| int(args, "default")).transpose()?.unwrap_or(0) as u16;
                    palette_rules.push(PaletteRule {
                        rect: rect4(field(args, "rect")?, "gs rect")?,
                        clear_rect: opt_field(args, "clear-rect").map(|a| rect4(a, "gs clear")).transpose()?,
                        kind: PaletteRuleKind::GradientSlider { label, stops, track_rect, default_permille },
                    });
                }
                _ => {}
            }
        }
        out.push(PlaneChromeBinding {
            id,
            identity,
            present_if_text: None,
            bands: vec![],
            buttons: vec![],
            suppress_bottom_right,
            window_range,
            pixel_guards,
            palette_rules,
        });
    }
    Ok(out)
}

/// The compat repo's own plane-chrome binding files for `platform`.
pub fn load_plane_chrome(platform: &str) -> Vec<PlaneChromeBinding> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bindings").join(platform);
    let mut out = vec![];
    for e in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "sexp") { out.extend(read_plane_chrome(&std::fs::read_to_string(&p).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display()))); }
    }
    out
}
