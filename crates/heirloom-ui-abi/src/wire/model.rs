//! Codecs for the model types (objects, windows, menus, controls, planes, requests, audio) and the wire refusal.
use super::*;

pub(super) fn expect_tag(reader: &mut Reader<'_>, expected: u8, what: &'static str) -> Result<(), WireError> {
    let tag = reader.u8()?;
    if tag == expected {
        Ok(())
    } else {
        Err(WireError::UnknownTag { what, tag })
    }
}

pub(super) fn put_object(out: &mut Vec<u8>, value: &UiObjectRef) {
    out.push(T_OBJECT);
    put_u32(out, value.id);
    put_u32(out, value.generation);
}

pub(super) fn get_object(reader: &mut Reader<'_>) -> Result<UiObjectRef, WireError> {
    expect_tag(reader, T_OBJECT, "UiObjectRef")?;
    Ok(UiObjectRef {
        id: reader.u32()?,
        generation: reader.u32()?,
    })
}

pub(super) fn put_token(out: &mut Vec<u8>, value: &CallbackToken) {
    out.push(T_TOKEN);
    put_u64(out, value.0);
}

pub(super) fn get_token(reader: &mut Reader<'_>) -> Result<CallbackToken, WireError> {
    expect_tag(reader, T_TOKEN, "CallbackToken")?;
    Ok(CallbackToken(reader.u64()?))
}

pub(super) fn put_rect(out: &mut Vec<u8>, value: &GuestRect) {
    out.push(T_RECT);
    put_i32(out, value.left);
    put_i32(out, value.top);
    put_i32(out, value.width);
    put_i32(out, value.height);
}

pub(super) fn get_rect(reader: &mut Reader<'_>) -> Result<GuestRect, WireError> {
    expect_tag(reader, T_RECT, "GuestRect")?;
    Ok(GuestRect {
        left: reader.i32()?,
        top: reader.i32()?,
        width: reader.i32()?,
        height: reader.i32()?,
    })
}

pub(super) fn put_point(out: &mut Vec<u8>, value: &GuestPoint) {
    out.push(T_POINT);
    put_i32(out, value.x);
    put_i32(out, value.y);
}

pub(super) fn get_point(reader: &mut Reader<'_>) -> Result<GuestPoint, WireError> {
    expect_tag(reader, T_POINT, "GuestPoint")?;
    Ok(GuestPoint {
        x: reader.i32()?,
        y: reader.i32()?,
    })
}

pub(super) fn put_modifiers(out: &mut Vec<u8>, value: &Modifiers) {
    out.push(T_MODIFIERS);
    out.push(value.0);
}

pub(super) fn get_modifiers(reader: &mut Reader<'_>) -> Result<Modifiers, WireError> {
    expect_tag(reader, T_MODIFIERS, "Modifiers")?;
    Ok(Modifiers(reader.u8()?))
}

pub(super) fn put_window_role(out: &mut Vec<u8>, value: &WindowRole) {
    out.push(match value {
        WindowRole::Document => 0,
        WindowRole::Dialog => 1,
        WindowRole::Utility => 2,
    });
}

pub(super) fn get_window_role(reader: &mut Reader<'_>) -> Result<WindowRole, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => WindowRole::Document,
        1 => WindowRole::Dialog,
        2 => WindowRole::Utility,
        _ => return Err(WireError::UnknownTag { what: "WindowRole", tag }),
    })
}

pub(super) fn put_window(out: &mut Vec<u8>, value: &WindowModel) {
    out.push(T_WINDOW);
    put_string(out, &value.title);
    put_rect(out, &value.rect);
    put_window_role(out, &value.role);
    put_bool(out, value.closable);
    put_bool(out, value.resizable);
    put_token(out, &value.token);
}

pub(super) fn get_window(reader: &mut Reader<'_>) -> Result<WindowModel, WireError> {
    expect_tag(reader, T_WINDOW, "WindowModel")?;
    Ok(WindowModel {
        title: reader.string()?,
        rect: get_rect(reader)?,
        role: get_window_role(reader)?,
        closable: reader.boolean()?,
        resizable: reader.boolean()?,
        token: get_token(reader)?,
    })
}

pub(super) fn put_menu_item(out: &mut Vec<u8>, value: &MenuItem) {
    out.push(T_MENU_ITEM);
    put_string(out, &value.label);
    put_bool(out, value.enabled);
    put_option(out, &value.shortcut, |out, value| put_u32(out, *value as u32));
    put_option(out, &value.check, |out, value| put_bool(out, *value));
    put_vec(out, &value.sub_items, put_menu_item);
    put_token(out, &value.token);
}

pub(super) fn get_menu_item(reader: &mut Reader<'_>) -> Result<MenuItem, WireError> {
    expect_tag(reader, T_MENU_ITEM, "MenuItem")?;
    Ok(MenuItem {
        label: reader.string()?,
        enabled: reader.boolean()?,
        shortcut: reader.option(|reader| reader.character())?,
        check: reader.option(|reader| reader.boolean())?,
        sub_items: reader.vec(20, get_menu_item)?,
        token: get_token(reader)?,
    })
}

pub(super) fn put_menu(out: &mut Vec<u8>, value: &Menu) {
    out.push(T_MENU);
    put_string(out, &value.title);
    put_bool(out, value.enabled);
    put_vec(out, &value.items, put_menu_item);
}

pub(super) fn get_menu(reader: &mut Reader<'_>) -> Result<Menu, WireError> {
    expect_tag(reader, T_MENU, "Menu")?;
    Ok(Menu {
        title: reader.string()?,
        enabled: reader.boolean()?,
        items: reader.vec(20, get_menu_item)?,
    })
}

pub(super) fn put_menu_model(out: &mut Vec<u8>, value: &MenuModel) {
    out.push(T_MENU_MODEL);
    put_vec(out, &value.menus, put_menu);
}

pub(super) fn get_menu_model(reader: &mut Reader<'_>) -> Result<MenuModel, WireError> {
    expect_tag(reader, T_MENU_MODEL, "MenuModel")?;
    Ok(MenuModel {
        menus: reader.vec(10, get_menu)?,
    })
}

pub(super) fn put_choice_style(out: &mut Vec<u8>, value: &ChoiceStyle) {
    out.push(match value {
        ChoiceStyle::Cycle => 0,
        ChoiceStyle::Radio => 1,
        ChoiceStyle::Popup => 2,
        ChoiceStyle::Tabs => 3,
    });
}

pub(super) fn get_choice_style(reader: &mut Reader<'_>) -> Result<ChoiceStyle, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => ChoiceStyle::Cycle,
        1 => ChoiceStyle::Radio,
        2 => ChoiceStyle::Popup,
        3 => ChoiceStyle::Tabs,
        _ => return Err(WireError::UnknownTag { what: "ChoiceStyle", tag }),
    })
}

pub(super) fn put_arrow_dir(out: &mut Vec<u8>, value: &ArrowDir) {
    out.push(match value {
        ArrowDir::Up => 0,
        ArrowDir::Down => 1,
        ArrowDir::Left => 2,
        ArrowDir::Right => 3,
    });
}

pub(super) fn get_arrow_dir(reader: &mut Reader<'_>) -> Result<ArrowDir, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => ArrowDir::Up,
        1 => ArrowDir::Down,
        2 => ArrowDir::Left,
        3 => ArrowDir::Right,
        _ => return Err(WireError::UnknownTag { what: "ArrowDir", tag }),
    })
}

pub(super) fn put_group_style(out: &mut Vec<u8>, value: &GroupStyle) {
    out.push(match value {
        GroupStyle::Plain => 0,
        GroupStyle::Recessed => 1,
        GroupStyle::Raised => 2,
    });
}

pub(super) fn get_group_style(reader: &mut Reader<'_>) -> Result<GroupStyle, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => GroupStyle::Plain,
        1 => GroupStyle::Recessed,
        2 => GroupStyle::Raised,
        _ => return Err(WireError::UnknownTag { what: "GroupStyle", tag }),
    })
}

pub(super) fn put_control_kind(out: &mut Vec<u8>, value: &ControlKind) {
    match value {
        ControlKind::Button => out.push(0),
        ControlKind::Checkbox { checked } => {
            out.push(1);
            put_bool(out, *checked);
        }
        ControlKind::String { text, max_len } => {
            out.push(2);
            put_string(out, text);
            put_u32(out, *max_len);
        }
        ControlKind::Slider { min, max, value } => {
            out.push(3);
            put_i32(out, *min);
            put_i32(out, *max);
            put_i32(out, *value);
        }
        ControlKind::Label { text, framed } => {
            out.push(4);
            put_string(out, text);
            put_bool(out, *framed);
        }
        ControlKind::ToggleButton { on } => {
            out.push(5);
            put_bool(out, *on);
        }
        ControlKind::ScrollBar { vertical, total, visible, top } => {
            out.push(6);
            put_bool(out, *vertical);
            put_u32(out, *total);
            put_u32(out, *visible);
            put_u32(out, *top);
        }
        ControlKind::Field { text, max_len, editable, integer } => {
            out.push(7);
            put_string(out, text);
            put_u32(out, *max_len);
            put_bool(out, *editable);
            put_bool(out, *integer);
        }
        ControlKind::Group { title, style } => {
            out.push(8);
            put_string(out, title);
            put_group_style(out, style);
        }
        ControlKind::ArrowButton { dir } => {
            out.push(9);
            put_arrow_dir(out, dir);
        }
        ControlKind::Hotspot => out.push(10),
        ControlKind::Choice { style, labels, selected } => {
            out.push(11);
            put_choice_style(out, style);
            put_vec(out, labels, |out, value| put_string(out, value));
            put_u32(out, *selected);
        }
        ControlKind::List { columns, rows, selected, top } => {
            out.push(12);
            put_vec(out, columns, |out, value| put_string(out, value));
            put_vec(out, rows, |out, row| {
                put_vec(out, row, |out, value| put_string(out, value));
            });
            put_option(out, selected, |out, value| put_u32(out, *value));
            put_u32(out, *top);
        }
        ControlKind::Gauge { value, max } => {
            out.push(13);
            put_u32(out, *value);
            put_u32(out, *max);
        }
        ControlKind::Swatches { colors, selected } => {
            out.push(14);
            put_vec(out, colors, |out, value| put_u32(out, *value));
            put_u32(out, *selected);
        }
    }
}

pub(super) fn get_rows(reader: &mut Reader<'_>) -> Result<Vec<Vec<String>>, WireError> {
    reader.vec(4, |reader| reader.vec(4, |reader| reader.string()))
}

pub(super) fn get_control_kind(reader: &mut Reader<'_>) -> Result<ControlKind, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => ControlKind::Button,
        1 => ControlKind::Checkbox { checked: reader.boolean()? },
        2 => ControlKind::String { text: reader.string()?, max_len: reader.u32()? },
        3 => ControlKind::Slider {
            min: reader.i32()?,
            max: reader.i32()?,
            value: reader.i32()?,
        },
        4 => ControlKind::Label { text: reader.string()?, framed: reader.boolean()? },
        5 => ControlKind::ToggleButton { on: reader.boolean()? },
        6 => ControlKind::ScrollBar {
            vertical: reader.boolean()?,
            total: reader.u32()?,
            visible: reader.u32()?,
            top: reader.u32()?,
        },
        7 => ControlKind::Field {
            text: reader.string()?,
            max_len: reader.u32()?,
            editable: reader.boolean()?,
            integer: reader.boolean()?,
        },
        8 => ControlKind::Group {
            title: reader.string()?,
            style: get_group_style(reader)?,
        },
        9 => ControlKind::ArrowButton { dir: get_arrow_dir(reader)? },
        10 => ControlKind::Hotspot,
        11 => ControlKind::Choice {
            style: get_choice_style(reader)?,
            labels: reader.vec(4, |reader| reader.string())?,
            selected: reader.u32()?,
        },
        12 => ControlKind::List {
            columns: reader.vec(4, |reader| reader.string())?,
            rows: get_rows(reader)?,
            selected: reader.option(|reader| reader.u32())?,
            top: reader.u32()?,
        },
        13 => ControlKind::Gauge { value: reader.u32()?, max: reader.u32()? },
        14 => ControlKind::Swatches {
            colors: reader.vec(4, |reader| reader.u32())?,
            selected: reader.u32()?,
        },
        _ => return Err(WireError::UnknownTag { what: "ControlKind", tag }),
    })
}

pub(super) fn put_control(out: &mut Vec<u8>, value: &ControlModel) {
    out.push(T_CONTROL);
    put_control_kind(out, &value.kind);
    put_string(out, &value.label);
    put_rect(out, &value.rect);
    put_bool(out, value.enabled);
    put_token(out, &value.token);
}

pub(super) fn get_control(reader: &mut Reader<'_>) -> Result<ControlModel, WireError> {
    expect_tag(reader, T_CONTROL, "ControlModel")?;
    Ok(ControlModel {
        kind: get_control_kind(reader)?,
        label: reader.string()?,
        rect: get_rect(reader)?,
        enabled: reader.boolean()?,
        token: get_token(reader)?,
    })
}

pub(super) fn put_control_value(out: &mut Vec<u8>, value: &ControlValue) {
    match value {
        ControlValue::Pressed => out.push(0),
        ControlValue::Checked(value) => {
            out.push(1);
            put_bool(out, *value);
        }
        ControlValue::Text(value) => {
            out.push(2);
            put_string(out, value);
        }
        ControlValue::Level(value) => {
            out.push(3);
            put_i32(out, *value);
        }
        ControlValue::Scroll { total, visible, top } => {
            out.push(4);
            put_u32(out, *total);
            put_u32(out, *visible);
            put_u32(out, *top);
        }
        ControlValue::Choice(value) => {
            out.push(5);
            put_u32(out, *value);
        }
        ControlValue::Rows { rows, selected, top } => {
            out.push(6);
            put_vec(out, rows, |out, row| {
                put_vec(out, row, |out, value| put_string(out, value));
            });
            put_option(out, selected, |out, value| put_u32(out, *value));
            put_u32(out, *top);
        }
        ControlValue::Cancelled => out.push(7),
    }
}

pub(super) fn get_control_value(reader: &mut Reader<'_>) -> Result<ControlValue, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => ControlValue::Pressed,
        1 => ControlValue::Checked(reader.boolean()?),
        2 => ControlValue::Text(reader.string()?),
        3 => ControlValue::Level(reader.i32()?),
        4 => ControlValue::Scroll {
            total: reader.u32()?,
            visible: reader.u32()?,
            top: reader.u32()?,
        },
        5 => ControlValue::Choice(reader.u32()?),
        6 => ControlValue::Rows {
            rows: get_rows(reader)?,
            selected: reader.option(|reader| reader.u32())?,
            top: reader.u32()?,
        },
        7 => ControlValue::Cancelled,
        _ => return Err(WireError::UnknownTag { what: "ControlValue", tag }),
    })
}

pub(super) fn put_pen_role(out: &mut Vec<u8>, value: &PenRole) {
    out.push(match value {
        PenRole::Surface => 0,
        PenRole::Text => 1,
        PenRole::TextMuted => 2,
        PenRole::Shine => 3,
        PenRole::Shadow => 4,
        PenRole::Fill => 5,
        PenRole::FillText => 6,
        PenRole::Accent => 7,
        PenRole::Highlight => 8,
        PenRole::Background => 9,
        PenRole::Selection => 10,
    });
}

pub(super) fn get_pen_role(reader: &mut Reader<'_>) -> Result<PenRole, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => PenRole::Surface,
        1 => PenRole::Text,
        2 => PenRole::TextMuted,
        3 => PenRole::Shine,
        4 => PenRole::Shadow,
        5 => PenRole::Fill,
        6 => PenRole::FillText,
        7 => PenRole::Accent,
        8 => PenRole::Highlight,
        9 => PenRole::Background,
        10 => PenRole::Selection,
        _ => return Err(WireError::UnknownTag { what: "PenRole", tag }),
    })
}

pub(super) fn put_font_role(out: &mut Vec<u8>, value: &FontRole) {
    out.push(match value {
        FontRole::Ui => 0,
        FontRole::Mono => 1,
        FontRole::Title => 2,
    });
}

pub(super) fn get_font_role(reader: &mut Reader<'_>) -> Result<FontRole, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => FontRole::Ui,
        1 => FontRole::Mono,
        2 => FontRole::Title,
        _ => return Err(WireError::UnknownTag { what: "FontRole", tag }),
    })
}

pub(super) fn put_icon_role(out: &mut Vec<u8>, value: &IconRole) {
    out.push(match value {
        IconRole::Disk => 0,
        IconRole::Volume => 1,
        IconRole::Drawer => 2,
        IconRole::Tool => 3,
        IconRole::Project => 4,
        IconRole::Trash => 5,
        IconRole::Device => 6,
        IconRole::Kick => 7,
    });
}

pub(super) fn get_icon_role(reader: &mut Reader<'_>) -> Result<IconRole, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => IconRole::Disk,
        1 => IconRole::Volume,
        2 => IconRole::Drawer,
        3 => IconRole::Tool,
        4 => IconRole::Project,
        5 => IconRole::Trash,
        6 => IconRole::Device,
        7 => IconRole::Kick,
        _ => return Err(WireError::UnknownTag { what: "IconRole", tag }),
    })
}

pub(super) fn put_draw_op(out: &mut Vec<u8>, value: &DrawOp) {
    match value {
        DrawOp::Fill { rect, pen_role } => {
            out.push(0);
            put_rect(out, rect);
            put_pen_role(out, pen_role);
        }
        DrawOp::Text { at, string, font_role, px, width, pen_role, bg_role } => {
            out.push(1);
            put_point(out, at);
            put_string(out, string);
            put_font_role(out, font_role);
            put_u16(out, *px);
            put_i32(out, *width);
            put_pen_role(out, pen_role);
            put_option(out, bg_role, put_pen_role);
        }
        DrawOp::Line { a, b, pen_role } => {
            out.push(2);
            put_point(out, a);
            put_point(out, b);
            put_pen_role(out, pen_role);
        }
        DrawOp::Bevel { rect, raised } => {
            out.push(3);
            put_rect(out, rect);
            put_bool(out, *raised);
        }
        DrawOp::Icon { rect, role } => {
            out.push(4);
            put_rect(out, rect);
            put_icon_role(out, role);
        }
        DrawOp::Pixels { rect, rgb } => {
            out.push(5);
            put_rect(out, rect);
            put_vec(out, rgb, |out, value| put_u32(out, *value));
        }
        DrawOp::Swatch { rect, rgb } => {
            out.push(6);
            put_rect(out, rect);
            put_u32(out, *rgb);
        }
        DrawOp::Face { rect, label, pressed, accent } => {
            out.push(7);
            put_rect(out, rect);
            put_string(out, label);
            put_bool(out, *pressed);
            put_option(out, accent, |out, value| put_u32(out, *value));
        }
    }
}

pub(super) fn get_draw_op(reader: &mut Reader<'_>) -> Result<DrawOp, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => DrawOp::Fill { rect: get_rect(reader)?, pen_role: get_pen_role(reader)? },
        1 => DrawOp::Text {
            at: get_point(reader)?,
            string: reader.string()?,
            font_role: get_font_role(reader)?,
            px: reader.u16()?,
            width: reader.i32()?,
            pen_role: get_pen_role(reader)?,
            bg_role: reader.option(get_pen_role)?,
        },
        2 => DrawOp::Line {
            a: get_point(reader)?,
            b: get_point(reader)?,
            pen_role: get_pen_role(reader)?,
        },
        3 => DrawOp::Bevel { rect: get_rect(reader)?, raised: reader.boolean()? },
        4 => DrawOp::Icon { rect: get_rect(reader)?, role: get_icon_role(reader)? },
        5 => DrawOp::Pixels {
            rect: get_rect(reader)?,
            rgb: reader.vec(4, |reader| reader.u32())?,
        },
        6 => DrawOp::Swatch { rect: get_rect(reader)?, rgb: reader.u32()? },
        7 => DrawOp::Face {
            rect: get_rect(reader)?,
            label: reader.string()?,
            pressed: reader.boolean()?,
            accent: reader.option(|reader| reader.u32())?,
        },
        _ => return Err(WireError::UnknownTag { what: "DrawOp", tag }),
    })
}

pub(super) fn put_plane(out: &mut Vec<u8>, value: &Plane) {
    out.push(T_PLANE);
    put_u32(out, value.id);
    put_rect(out, &value.rect);
    put_vec(out, &value.ops, put_draw_op);
}

pub(super) fn get_plane(reader: &mut Reader<'_>) -> Result<Plane, WireError> {
    expect_tag(reader, T_PLANE, "Plane")?;
    Ok(Plane {
        id: reader.u32()?,
        rect: get_rect(reader)?,
        ops: reader.vec(19, get_draw_op)?,
    })
}

pub(super) fn put_request_kind(out: &mut Vec<u8>, value: &RequestKind) {
    out.push(match value {
        RequestKind::Unspecified => 0,
        RequestKind::Note => 1,
        RequestKind::Caution => 2,
        RequestKind::Stop => 3,
    });
}

pub(super) fn get_request_kind(reader: &mut Reader<'_>) -> Result<RequestKind, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => RequestKind::Unspecified,
        1 => RequestKind::Note,
        2 => RequestKind::Caution,
        3 => RequestKind::Stop,
        _ => return Err(WireError::UnknownTag { what: "RequestKind", tag }),
    })
}

pub(super) fn put_request(out: &mut Vec<u8>, value: &RequestModel) {
    out.push(T_REQUEST);
    put_string(out, &value.title);
    put_string(out, &value.text);
    put_vec(out, &value.buttons, |out, value| put_string(out, value));
    put_option(out, &value.default, |out, value| put_u32(out, *value));
    put_option(out, &value.cancel, |out, value| put_u32(out, *value));
    put_request_kind(out, &value.kind);
}

pub(super) fn get_request(reader: &mut Reader<'_>) -> Result<RequestModel, WireError> {
    expect_tag(reader, T_REQUEST, "RequestModel")?;
    Ok(RequestModel {
        title: reader.string()?,
        text: reader.string()?,
        buttons: reader.vec(4, |reader| reader.string())?,
        default: reader.option(|reader| reader.u32())?,
        cancel: reader.option(|reader| reader.u32())?,
        kind: get_request_kind(reader)?,
    })
}

pub(super) fn put_audio_format(out: &mut Vec<u8>, value: &AudioFormat) {
    out.push(match value {
        AudioFormat::U8 => 0,
        AudioFormat::S16Le => 1,
    });
}

pub(super) fn get_audio_format(reader: &mut Reader<'_>) -> Result<AudioFormat, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => AudioFormat::U8,
        1 => AudioFormat::S16Le,
        _ => return Err(WireError::UnknownTag { what: "AudioFormat", tag }),
    })
}

pub(super) fn put_audio(out: &mut Vec<u8>, value: &AudioStreamModel) {
    out.push(T_AUDIO);
    put_u32(out, value.rate_hz);
    out.push(value.channels);
    put_audio_format(out, &value.format);
    put_string(out, &value.label);
}

pub(super) fn get_audio(reader: &mut Reader<'_>) -> Result<AudioStreamModel, WireError> {
    expect_tag(reader, T_AUDIO, "AudioStreamModel")?;
    Ok(AudioStreamModel {
        rate_hz: reader.u32()?,
        channels: reader.u8()?,
        format: get_audio_format(reader)?,
        label: reader.string()?,
    })
}

pub(super) fn put_refusal(out: &mut Vec<u8>, value: &UiRefusal) {
    match value {
        UiRefusal::UnknownObject(object) => {
            out.push(0);
            put_object(out, object);
        }
        UiRefusal::StaleGeneration(object) => {
            out.push(1);
            put_object(out, object);
        }
        UiRefusal::InvalidGeometry(reason) => {
            out.push(2);
            put_string(out, reason);
        }
        UiRefusal::Unsupported(reason) => {
            out.push(3);
            put_string(out, reason);
        }
    }
}

/// [`Frame::Refused`]'s reason as it crosses the wire: the shape of [`UiRefusal`] with owned text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireRefusal {
    UnknownObject(UiObjectRef),
    StaleGeneration(UiObjectRef),
    InvalidGeometry(String),
    Unsupported(String),
}

impl WireRefusal {
    /// The wire form of a host-side refusal.
    pub fn from_ui(why: &UiRefusal) -> Self {
        match why {
            UiRefusal::UnknownObject(o) => Self::UnknownObject(*o),
            UiRefusal::StaleGeneration(o) => Self::StaleGeneration(*o),
            UiRefusal::InvalidGeometry(r) => Self::InvalidGeometry(String::from(*r)),
            UiRefusal::Unsupported(r) => Self::Unsupported(String::from(*r)),
        }
    }
    /// The engine-side [`UiRefusal`] of the same kind; the host's own text travels in the frame (log it from there).
    pub fn to_ui(&self) -> UiRefusal {
        match self {
            Self::UnknownObject(o) => UiRefusal::UnknownObject(*o),
            Self::StaleGeneration(o) => UiRefusal::StaleGeneration(*o),
            Self::InvalidGeometry(_) => UiRefusal::InvalidGeometry("the host refused the geometry"),
            Self::Unsupported(_) => UiRefusal::Unsupported("the host refused the call"),
        }
    }
}

pub(super) fn get_refusal(reader: &mut Reader<'_>) -> Result<WireRefusal, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => WireRefusal::UnknownObject(get_object(reader)?),
        1 => WireRefusal::StaleGeneration(get_object(reader)?),
        2 => WireRefusal::InvalidGeometry(reader.string()?),
        3 => WireRefusal::Unsupported(reader.string()?),
        _ => return Err(WireError::UnknownTag { what: "UiRefusal", tag }),
    })
}
