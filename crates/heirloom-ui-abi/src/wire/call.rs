//! Codecs for every UiCall and GuestUiEvent.
use super::*;

pub(super) fn put_intent(out: &mut Vec<u8>, value: &Intent) {
    match value {
        Intent::Activate => out.push(0),
        Intent::SetValue(value) => {
            out.push(1);
            put_control_value(out, value);
        }
        Intent::SelectRow(row) => {
            out.push(2);
            put_u32(out, *row);
        }
        Intent::ScrollTo(top) => {
            out.push(3);
            put_u32(out, *top);
        }
        Intent::CommitText(text) => {
            out.push(4);
            put_string(out, text);
        }
        Intent::Press => out.push(5),
        Intent::Release { inside } => {
            out.push(6);
            put_bool(out, *inside);
        }
        Intent::Drag(value) => {
            out.push(7);
            put_control_value(out, value);
        }
    }
}

pub(super) fn get_intent(reader: &mut Reader<'_>) -> Result<Intent, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => Intent::Activate,
        1 => Intent::SetValue(get_control_value(reader)?),
        2 => Intent::SelectRow(reader.u32()?),
        3 => Intent::ScrollTo(reader.u32()?),
        4 => Intent::CommitText(reader.string()?),
        5 => Intent::Press,
        6 => Intent::Release { inside: reader.boolean()? },
        7 => Intent::Drag(get_control_value(reader)?),
        _ => return Err(WireError::UnknownTag { what: "Intent", tag }),
    })
}

pub(super) fn put_pointer_button(out: &mut Vec<u8>, value: &PointerButton) {
    out.push(match value {
        PointerButton::None => 0,
        PointerButton::Primary => 1,
        PointerButton::Secondary => 2,
        PointerButton::Middle => 3,
    });
}

pub(super) fn get_pointer_button(reader: &mut Reader<'_>) -> Result<PointerButton, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => PointerButton::None,
        1 => PointerButton::Primary,
        2 => PointerButton::Secondary,
        3 => PointerButton::Middle,
        _ => return Err(WireError::UnknownTag { what: "PointerButton", tag }),
    })
}

pub(super) fn put_pointer_phase(out: &mut Vec<u8>, value: &PointerPhase) {
    out.push(match value {
        PointerPhase::Down => 0,
        PointerPhase::Move => 1,
        PointerPhase::Up => 2,
    });
}

pub(super) fn get_pointer_phase(reader: &mut Reader<'_>) -> Result<PointerPhase, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => PointerPhase::Down,
        1 => PointerPhase::Move,
        2 => PointerPhase::Up,
        _ => return Err(WireError::UnknownTag { what: "PointerPhase", tag }),
    })
}

pub(super) fn put_call(out: &mut Vec<u8>, call: &UiCall) {
    match call {
        UiCall::OpenWindow(model) => {
            out.push(0);
            put_window(out, model);
        }
        UiCall::CloseWindow(win) => {
            out.push(1);
            put_object(out, win);
        }
        UiCall::SetWindowTitle(win, title) => {
            out.push(2);
            put_object(out, win);
            put_string(out, title);
        }
        UiCall::HideWindow(win) => {
            out.push(3);
            put_object(out, win);
        }
        UiCall::ShowWindow(win) => {
            out.push(4);
            put_object(out, win);
        }
        UiCall::SetMenuStrip(win, menu) => {
            out.push(5);
            put_object(out, win);
            put_menu_model(out, menu);
        }
        UiCall::ClearMenuStrip(win) => {
            out.push(6);
            put_object(out, win);
        }
        UiCall::AddControl(win, control) => {
            out.push(7);
            put_object(out, win);
            put_control(out, control);
        }
        UiCall::RemoveControl(win, control) => {
            out.push(8);
            put_object(out, win);
            put_object(out, control);
        }
        UiCall::SetControl(win, control, value) => {
            out.push(9);
            put_object(out, win);
            put_object(out, control);
            put_control_value(out, value);
        }
        UiCall::SetControlRect(win, control, rect) => {
            out.push(10);
            put_object(out, win);
            put_object(out, control);
            put_rect(out, rect);
        }
        UiCall::SetControlEnabled(win, control, enabled) => {
            out.push(11);
            put_object(out, win);
            put_object(out, control);
            put_bool(out, *enabled);
        }
        UiCall::PresentContent { win, damage, width, height, hash } => {
            out.push(12);
            put_object(out, win);
            put_rect(out, damage);
            put_i32(out, *width);
            put_i32(out, *height);
            put_u64(out, *hash);
        }
        UiCall::PresentPlane(win, plane) => {
            out.push(13);
            put_object(out, win);
            put_plane(out, plane);
        }
        UiCall::Request(win, model) => {
            out.push(14);
            put_option(out, win, put_object);
            put_request(out, model);
        }
        UiCall::CloseRequest(request) => {
            out.push(15);
            put_object(out, request);
        }
        UiCall::OpenAudio(model) => {
            out.push(16);
            put_audio(out, model);
        }
        UiCall::QueueAudio(stream, bytes) => {
            out.push(17);
            put_object(out, stream);
            put_usize(out, *bytes);
        }
        UiCall::QueueAudioLooped(stream, bytes, loop_start, loop_end) => {
            out.push(18);
            put_object(out, stream);
            put_usize(out, *bytes);
            put_u32(out, *loop_start);
            put_u32(out, *loop_end);
        }
        UiCall::FlushAudio(stream) => {
            out.push(19);
            put_object(out, stream);
        }
        UiCall::PauseAudio(stream, paused) => {
            out.push(20);
            put_object(out, stream);
            put_bool(out, *paused);
        }
        UiCall::SetAudioVolume(stream, left, right) => {
            out.push(21);
            put_object(out, stream);
            put_u16(out, *left);
            put_u16(out, *right);
        }
        UiCall::CloseAudio(stream) => {
            out.push(22);
            put_object(out, stream);
        }
    }
}

pub(super) fn get_call(reader: &mut Reader<'_>) -> Result<UiCall, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => UiCall::OpenWindow(get_window(reader)?),
        1 => UiCall::CloseWindow(get_object(reader)?),
        2 => UiCall::SetWindowTitle(get_object(reader)?, reader.string()?),
        3 => UiCall::HideWindow(get_object(reader)?),
        4 => UiCall::ShowWindow(get_object(reader)?),
        5 => UiCall::SetMenuStrip(get_object(reader)?, get_menu_model(reader)?),
        6 => UiCall::ClearMenuStrip(get_object(reader)?),
        7 => UiCall::AddControl(get_object(reader)?, get_control(reader)?),
        8 => UiCall::RemoveControl(get_object(reader)?, get_object(reader)?),
        9 => UiCall::SetControl(
            get_object(reader)?,
            get_object(reader)?,
            get_control_value(reader)?,
        ),
        10 => UiCall::SetControlRect(
            get_object(reader)?,
            get_object(reader)?,
            get_rect(reader)?,
        ),
        11 => UiCall::SetControlEnabled(
            get_object(reader)?,
            get_object(reader)?,
            reader.boolean()?,
        ),
        12 => UiCall::PresentContent {
            win: get_object(reader)?,
            damage: get_rect(reader)?,
            width: reader.i32()?,
            height: reader.i32()?,
            hash: reader.u64()?,
        },
        13 => UiCall::PresentPlane(get_object(reader)?, get_plane(reader)?),
        14 => UiCall::Request(
            reader.option(get_object)?,
            get_request(reader)?,
        ),
        15 => UiCall::CloseRequest(get_object(reader)?),
        16 => UiCall::OpenAudio(get_audio(reader)?),
        17 => UiCall::QueueAudio(get_object(reader)?, reader.usize()?),
        18 => UiCall::QueueAudioLooped(
            get_object(reader)?,
            reader.usize()?,
            reader.u32()?,
            reader.u32()?,
        ),
        19 => UiCall::FlushAudio(get_object(reader)?),
        20 => UiCall::PauseAudio(get_object(reader)?, reader.boolean()?),
        21 => UiCall::SetAudioVolume(get_object(reader)?, reader.u16()?, reader.u16()?),
        22 => UiCall::CloseAudio(get_object(reader)?),
        _ => return Err(WireError::UnknownTag { what: "UiCall", tag }),
    })
}

pub(super) fn put_event(out: &mut Vec<u8>, event: &GuestUiEvent) {
    match event {
        GuestUiEvent::CloseRequested { win, token } => {
            out.push(0);
            put_object(out, win);
            put_token(out, token);
        }
        GuestUiEvent::MenuCommand { win, token } => {
            out.push(1);
            put_object(out, win);
            put_token(out, token);
        }
        GuestUiEvent::Resized { win, rect } => {
            out.push(2);
            put_object(out, win);
            put_rect(out, rect);
        }
        GuestUiEvent::Intent { win, control, token, generation, intent } => {
            out.push(3);
            put_object(out, win);
            put_object(out, control);
            put_token(out, token);
            put_u32(out, *generation);
            put_intent(out, intent);
        }
        GuestUiEvent::Pointer { win, x, y, button, phase, clicks } => {
            out.push(4);
            put_object(out, win);
            put_i32(out, *x);
            put_i32(out, *y);
            put_pointer_button(out, button);
            put_pointer_phase(out, phase);
            out.push(*clicks);
        }
        GuestUiEvent::Key { win, code, text, modifiers, down } => {
            out.push(5);
            put_object(out, win);
            put_u32(out, *code);
            put_string(out, text);
            put_modifiers(out, modifiers);
            put_bool(out, *down);
        }
        GuestUiEvent::RequestAnswered { req, button } => {
            out.push(6);
            put_object(out, req);
            put_u32(out, *button);
        }
    }
}

pub(super) fn get_event(reader: &mut Reader<'_>) -> Result<GuestUiEvent, WireError> {
    let tag = reader.u8()?;
    Ok(match tag {
        0 => GuestUiEvent::CloseRequested {
            win: get_object(reader)?,
            token: get_token(reader)?,
        },
        1 => GuestUiEvent::MenuCommand {
            win: get_object(reader)?,
            token: get_token(reader)?,
        },
        2 => GuestUiEvent::Resized {
            win: get_object(reader)?,
            rect: get_rect(reader)?,
        },
        3 => GuestUiEvent::Intent {
            win: get_object(reader)?,
            control: get_object(reader)?,
            token: get_token(reader)?,
            generation: reader.u32()?,
            intent: get_intent(reader)?,
        },
        4 => GuestUiEvent::Pointer {
            win: get_object(reader)?,
            x: reader.i32()?,
            y: reader.i32()?,
            button: get_pointer_button(reader)?,
            phase: get_pointer_phase(reader)?,
            clicks: reader.u8()?,
        },
        5 => GuestUiEvent::Key {
            win: get_object(reader)?,
            code: reader.u32()?,
            text: reader.string()?,
            modifiers: get_modifiers(reader)?,
            down: reader.boolean()?,
        },
        6 => GuestUiEvent::RequestAnswered {
            req: get_object(reader)?,
            button: reader.u32()?,
        },
        _ => return Err(WireError::UnknownTag { what: "GuestUiEvent", tag }),
    })
}
