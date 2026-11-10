use super::{Probe, legacy_warp, lua_warp, parse_modifiers, parse_monitors, parse_position, pixel};
use crate::{DisplayBounds, KeyModifiers};

#[test]
#[ignore = "requires a live Hyprland session"]
fn live_hyprland_probe_answers() {
    let mut probe = Probe::connect().expect("a live Hyprland IPC socket");
    assert!(
        probe.reports_modifiers(),
        "Hyprland's Lua API provides the Control guard"
    );
    assert!(
        !probe.displays().is_empty(),
        "Hyprland reports at least one monitor"
    );
    let position = probe.state().expect("Hyprland reports the cursor").position;
    probe.set_modifiers_required(true);
    assert!(
        probe.state().is_some(),
        "Hyprland answers the modifier query"
    );
    probe
        .warp(position)
        .expect("Hyprland accepts a warp to the current position");
}

#[test]
fn modifier_bits_parse_in_shift_control_option_command_order() {
    assert_eq!(
        parse_modifiers("1010"),
        Some(KeyModifiers {
            shift: true,
            control: false,
            option: true,
            command: false,
        })
    );
    assert_eq!(
        parse_modifiers("0101\n"),
        Some(KeyModifiers {
            shift: false,
            control: true,
            option: false,
            command: true,
        })
    );
    assert!(parse_modifiers("").is_none());
    assert!(parse_modifiers("101").is_none());
    assert!(parse_modifiers("10x0").is_none());
}

#[test]
fn cursor_positions_parse_with_optional_space_and_sign() {
    assert_eq!(
        parse_position("1284, 2287").map(|point| (point.x, point.y)),
        Some((1284.0, 2287.0))
    );
    assert_eq!(
        parse_position(" -10 , 20 \n").map(|point| (point.x, point.y)),
        Some((-10.0, 20.0))
    );
    assert_eq!(
        parse_position("1.5,2.5").map(|point| (point.x, point.y)),
        Some((1.5, 2.5))
    );
    assert!(parse_position("1284").is_none());
    assert!(parse_position("").is_none());
    assert!(parse_position("nan, 1").is_none());
    assert!(parse_position("1, inf").is_none());
}

#[test]
fn monitors_are_converted_to_logical_layout_units() {
    let json = r#"[
        {"x": 0, "y": 1440, "width": 3440, "height": 1440, "scale": 1.0, "transform": 0, "disabled": false},
        {"x": -1920, "y": 0, "width": 3840, "height": 2160, "scale": 2.0, "transform": 0, "disabled": false},
        {"x": 3440, "y": 0, "width": 2560, "height": 1440, "scale": 1.0, "transform": 1, "disabled": false},
        {"x": 0, "y": 0, "width": 1920, "height": 1080, "scale": 1.0, "transform": 0, "disabled": true}
    ]"#;
    assert_eq!(
        parse_monitors(json),
        Some(vec![
            DisplayBounds {
                x: 0.0,
                y: 1440.0,
                width: 3440.0,
                height: 1440.0,
            },
            DisplayBounds {
                x: -1920.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            DisplayBounds {
                x: 3440.0,
                y: 0.0,
                width: 1440.0,
                height: 2560.0,
            },
        ])
    );
}

#[test]
fn malformed_monitor_json_is_rejected() {
    assert!(parse_monitors("").is_none());
    assert!(parse_monitors("[] trailing").is_none());
    assert!(parse_monitors("{}").is_none());
}

#[test]
fn invalid_scale_and_size_are_ignored() {
    let json = r#"[
        {"x": 0, "y": 0, "width": 1920, "height": 1080, "scale": 0.0, "transform": 0, "disabled": false},
        {"x": 0, "y": 0, "width": 0, "height": 1080, "scale": 1.0, "transform": 0, "disabled": false}
    ]"#;
    assert_eq!(
        parse_monitors(json),
        Some(vec![DisplayBounds {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        }])
    );
}

#[test]
fn warp_coordinates_round_and_saturate() {
    assert_eq!(pixel(959.5), 960);
    assert_eq!(pixel(-10.6), -11);
    assert_eq!(pixel(1e12), i32::MAX);
    assert_eq!(pixel(-1e12), i32::MIN);
    assert_eq!(pixel(f64::NAN), 0);
}

#[test]
fn both_hyprland_dispatch_syntaxes_are_well_formed() {
    assert_eq!(legacy_warp(10, -20), "dispatch movecursor 10 -20");
    assert_eq!(
        lua_warp(10, -20),
        "eval hl.dispatch(hl.dsp.cursor.move({x=10, y=-20}))"
    );
}
