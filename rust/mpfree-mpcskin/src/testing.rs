//! A small skin for tests here and in the app: no pictures, nothing from
//! any real plugin.

use std::path::{Path, PathBuf};

/// Two knob slots (each a component with the usual Q-Link and overlay
/// actions) and an on/off button, on one tab.
pub const SKIN: &str = r#"{"pageData": {"version": 1,
      "componentDefinitions": {"version": 2, "importFiles": [],
        "localComponentDefinitions": [
          {"key": "slot", "value": {"version": 2,
            "actions": [
              {"version": 2, "onAction": "Mouse Down", "handler": "Q-Link", "handleName": "Data", "additionalData": ""},
              {"version": 2, "onAction": "Double Click", "handler": "Show Overlay", "handleName": "Data", "additionalData": "knob overlay"},
              {"version": 2, "onAction": "Enter Pressed", "handler": "Show Overlay", "handleName": "Data", "additionalData": "knob overlay"}],
            "backgroundData": {"version": 1, "focussed": {"version": 1, "colour": "ff303030", "image": ""}, "unfocussed": {"version": 1, "colour": "ff101010", "image": ""}},
            "ignoreMousePresses": false, "disableCoarseDataWheel": false,
            "componentsData": [{"version": 1,
              "componentData": {"version": 1, "name": "k", "type": "Knob", "data": {"version": 2, "knobType": "GenericNoLabel", "handleName": "Data"}},
              "handle remapping": {"version": 1, "map": []},
              "bounds": {"version": 1, "acceptsHWFocus": "No", "showWhenDataModelInvalid": "Hide", "whenVisible": "Always", "boundsType": "Absolute", "bounds": "0 0 200 200"}}]}},
          {"key": "page", "value": {"version": 2, "actions": [],
            "backgroundData": {"version": 1, "focussed": {"version": 1, "colour": "ff000000", "image": ""}, "unfocussed": {"version": 1, "colour": "ff000000", "image": ""}},
            "ignoreMousePresses": false, "disableCoarseDataWheel": false,
            "componentsData": [
              {"version": 1, "componentData": {"version": 1, "name": "a", "type": "slot", "data": {}},
               "handle remapping": {"version": 1, "map": [{"key": "Data", "value": "Parameter 0"}]},
               "bounds": {"version": 1, "acceptsHWFocus": "Yes", "showWhenDataModelInvalid": "Hide", "whenVisible": "Always", "boundsType": "Absolute", "bounds": "100 100 200 200"}},
              {"version": 1, "componentData": {"version": 1, "name": "b", "type": "slot", "data": {}},
               "handle remapping": {"version": 1, "map": [{"key": "Data", "value": "Parameter 1"}]},
               "bounds": {"version": 1, "acceptsHWFocus": "Yes", "showWhenDataModelInvalid": "Hide", "whenVisible": "Always", "boundsType": "Absolute", "bounds": "400 100 200 200"}},
              {"version": 1, "componentData": {"version": 1, "name": "on", "type": "Button", "data": {"version": 1, "onImage": "", "offImage": "", "buttonId": 1, "numButtonsInGroup": 1, "handleName": "Parameter 2"}},
               "handle remapping": {"version": 1, "map": []},
               "bounds": {"version": 1, "acceptsHWFocus": "No", "showWhenDataModelInvalid": "Hide", "whenVisible": "Always", "boundsType": "Absolute", "bounds": "700 100 100 60"}}]}}]},
      "replacementStrings": [],
      "info": {"version": 1, "type": "CompleteDescription"},
      "tabs": [{"version": 3, "tabName": "MAIN", "fnKeyIndex": 0, "fnKeySubIndex": 0, "qlinkBoundsData": ["100 100 500 200"], "componentName": "page", "initialSize": "0 0 1280 628", "scale": 1.0}]}}"#;

/// Writes a plugin folder holding only [`SKIN`] (as `Plugin Skins/TUI.json`)
/// in `dir`, and gives the folder.
pub fn write_plugin(dir: &Path) -> std::io::Result<PathBuf> {
    let folder = dir.join("MPFree Tests - VST - Test Skin");
    let skins = folder.join("Plugin Skins");
    std::fs::create_dir_all(&skins)?;
    std::fs::write(skins.join("TUI.json"), SKIN)?;
    Ok(folder)
}
