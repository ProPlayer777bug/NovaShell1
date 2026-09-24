//! Controller input (Linux).
//!
//! Uses `gilrs` (evdev under the hood) and maps logical gamepad buttons to
//! high-level NovaShell actions. The mapping is configurable in
//! `config.json` under `controller.mapping`.
//!
//! {button: "south"} -> {action: "confirm"}

use crate::settings::ControllerConfig;
use gilrs::{Axis, Button, EventType, Gilrs};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// High-level actions the UI understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    Menu,
    Context,
    Detail,
    TabPrev,
    TabNext,
    QuickMenu,
    Screenshot,
    None,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Up => "up",
            Action::Down => "down",
            Action::Left => "left",
            Action::Right => "right",
            Action::Confirm => "confirm",
            Action::Back => "back",
            Action::Menu => "menu",
            Action::Context => "context",
            Action::Detail => "detail",
            Action::TabPrev => "tab_prev",
            Action::TabNext => "tab_next",
            Action::QuickMenu => "quick_menu",
            Action::Screenshot => "screenshot",
            Action::None => "none",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub pad: usize,
    pub name: String,
    pub action: Action,
    pub pressed: bool,
    pub is_axis: bool,
}

/// Start the controller thread. Returns a channel that receives press events.
pub fn spawn(cfg: ControllerConfig) -> Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("novashell-input".into())
        .spawn(move || run_loop(tx, cfg))
        .ok();
    rx
}

fn run_loop(tx: mpsc::Sender<Event>, cfg: ControllerConfig) {
    if !cfg.enabled {
        return;
    }
    let mut gilrs = match Gilrs::new() {
        Ok(g) => g,
        Err(e) => {
            log::warn!("no gamepad support: {e}");
            return;
        }
    };
    let mapping = cfg.mapping.clone();
    let mut prev_axis: HashMap<(usize, gilrs::Axis), f32> = HashMap::new();

    // Report already-connected pads on boot.
    for (id, pad) in gilrs.gamepads() {
        let _ = tx.send(Event {
            pad: usize::from(id),
            name: pad.name().to_string(),
            action: Action::None,
            pressed: true,
            is_axis: false,
        });
    }

    loop {
        while let Some(ev) = gilrs.next_event() {
            let id = usize::from(ev.id);
            match ev.event {
EventType::Connected => {
                    let pad = gilrs.gamepad(ev.id);
                    let _ = tx.send(Event {
                        pad: id,
                        name: pad.name().to_string(),
                        action: Action::None,
                        pressed: true,
                        is_axis: false,
                    });
                }
                EventType::Disconnected => {
                    let _ = tx.send(Event {
                        pad: id,
                        name: String::new(),
                        action: Action::None,
                        pressed: false,
                        is_axis: false,
                    });
                }
                EventType::ButtonPressed(button, _) => {
                    if let Some(action) = map_button(button, &mapping) {
                        if action != Action::None {
                            let _ = tx.send(Event {
                                pad: id,
                                name: String::new(),
                                action,
                                pressed: true,
                                is_axis: false,
                            });
                        }
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(action) = map_button(button, &mapping) {
                        let _ = tx.send(Event {
                            pad: id,
                            name: String::new(),
                            action,
                            pressed: false,
                            is_axis: false,
                        });
                    }
                }
                EventType::AxisChanged(axis, val, _) => {
                    handle_axis(id, axis, val, &mut prev_axis, &tx);
                }
                _ => {
                    let _ = ();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

const AXIS_THRESHOLD: f32 = 0.6;

fn handle_axis(
    id: usize,
    axis: Axis,
    val: f32,
    prev: &mut HashMap<(usize, gilrs::Axis), f32>,
    tx: &mpsc::Sender<Event>,
) {
    let key = (id, axis);
    let before = prev.get(&key).copied().unwrap_or(0.0);
    prev.insert(key, val);

    let dir = match axis {
        Axis::LeftStickX => Some(if val > AXIS_THRESHOLD {
            Action::Right
        } else if val < -AXIS_THRESHOLD {
            Action::Left
        } else {
            Action::None
        }),
        Axis::LeftStickY => Some(if val < -AXIS_THRESHOLD {
            Action::Up
        } else if val > AXIS_THRESHOLD {
            Action::Down
        } else {
            Action::None
        }),
        _ => None,
    };

    if let Some(action) = dir {
        let crossed_in = action != Action::None && before.abs() < AXIS_THRESHOLD;
        if crossed_in {
            let _ = tx.send(Event {
                pad: id,
                name: String::new(),
                action,
                pressed: true,
                is_axis: true,
            });
        }
    }
}

/// Map a gilrs logical button name through the user mapping table.
fn map_button(button: Button, mapping: &BTreeMap<String, String>) -> Option<Action> {
    let name = match button {
        Button::South => "south",
        Button::East => "east",
        Button::North => "north",
        Button::West => "west",
        Button::Start => "start",
        Button::Select => "select",
        Button::Mode => "guide",
        Button::LeftTrigger => "l2",
        Button::RightTrigger => "r2",
        Button::LeftTrigger2 => "l1",
        Button::RightTrigger2 => "r1",
        Button::LeftThumb => "l3",
        Button::RightThumb => "r3",
        Button::DPadUp => "dpad_up",
        Button::DPadDown => "dpad_down",
        Button::DPadLeft => "dpad_left",
        Button::DPadRight => "dpad_right",
        _ => "unknown",
    };
    if name == "unknown" {
        return None;
    }
    let action_name = mapping.get(name).cloned().unwrap_or_else(|| "none".into());
    Some(action_from_str(&action_name))
}

pub fn action_from_str(s: &str) -> Action {
    match s {
        "up" => Action::Up,
        "down" => Action::Down,
        "left" => Action::Left,
        "right" => Action::Right,
        "confirm" => Action::Confirm,
        "back" => Action::Back,
        "menu" => Action::Menu,
        "context" => Action::Context,
        "detail" => Action::Detail,
        "tab_prev" => Action::TabPrev,
        "tab_next" => Action::TabNext,
        "quick_menu" => Action::QuickMenu,
        "screenshot" => Action::Screenshot,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_map_matches_core_actions() {
        let mut m = BTreeMap::new();
        for (k, v) in crate::settings::DEFAULT_CONTROLLER_MAP {
            m.insert(k.to_string(), v.to_string());
        }
        assert_eq!(map_button(Button::South, &m), Some(Action::Confirm));
        assert_eq!(map_button(Button::East, &m), Some(Action::Back));
        assert_eq!(map_button(Button::DPadLeft, &m), Some(Action::Left));
        assert_eq!(map_button(Button::Start, &m), Some(Action::Menu));
    }
}