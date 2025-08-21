use std::{fmt::Display, hash::Hash, rc::Rc};

use dioxus::{
    events::{Modifiers, ModifiersInteraction},
    html::KeyboardData,
    logger::tracing::error,
};
use serde::{Deserialize, Serialize};

use super::key_strike::KeyStrike;

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord,
)]
#[serde(try_from = "String", into = "String")]
pub struct KeyCombo {
    modifiers: KeyModifiers,
    key: KeyStrike,
}

impl Display for KeyCombo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let modif = self.modifiers.to_string();
        let key = self.key.to_string();

        write!(f, "{}", [modif, key].join(" & "))
    }
}

impl TryFrom<String> for KeyCombo {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let splits = value.split("&").collect::<Vec<_>>();
        match splits.len() {
            0 => Err("No Keys found here".to_string()),
            1 => match KeyStrike::try_from(splits.first().unwrap().trim().to_string()) {
                Ok(ks) => Ok(KeyCombo {
                    modifiers: KeyModifiers::default(),
                    key: ks,
                }),
                Err(e) => Err(e),
            },
            2 => {
                let m = splits.first().unwrap().trim().to_string();
                let k = splits.last().unwrap().trim().to_string();

                match (KeyModifiers::try_from(m), KeyStrike::try_from(k)) {
                    (Ok(modifiers), Ok(key)) => Ok(KeyCombo { modifiers, key }),
                    (Ok(_), Err(e)) => Err(e),
                    (Err(e), Ok(_)) => Err(e),
                    (Err(em), Err(ek)) => Err(format!("{} - {}", em, ek)),
                }
            }
            _ => Err(format!("This is a non valid combination, only one key and a modifier combination is allowed: {}", value))
        }
    }
}

impl From<KeyCombo> for String {
    fn from(value: KeyCombo) -> Self {
        value.to_string()
    }
}

impl TryFrom<KeyboardData> for KeyCombo {
    type Error = String;

    fn try_from(value: KeyboardData) -> Result<Self, Self::Error> {
        let key: KeyStrike = value.key().into();
        let modifiers: KeyModifiers = value.modifiers().into();

        if key == KeyStrike::Unknown {
            Err(format!("Unknown Key: {}", value.key()))
        } else {
            Ok(KeyCombo { modifiers, key })
        }
    }
}

impl From<Rc<KeyboardData>> for KeyCombo {
    fn from(value: Rc<KeyboardData>) -> Self {
        let key: KeyStrike = value.key().into();
        let modifiers: KeyModifiers = value.modifiers().into();

        if key == KeyStrike::Unknown {
            error!("Unknown Key: {}", value.key());
            KeyCombo::default()
        } else {
            KeyCombo { modifiers, key }
        }
    }
}

impl KeyCombo {
    pub fn new(modifiers: KeyModifiers, key: KeyStrike) -> Self {
        Self { modifiers, key }
    }
}

/// Pressed modifier keys.
///
/// Specification:
/// <https://w3c.github.io/uievents-key/#keys-modifier>
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord,
)]
#[serde(try_from = "String", into = "String")]
pub struct KeyModifiers {
    alt: bool,
    ctrl: bool,
    cmd: bool,
    shift: bool,
}

// For compatibility
const META: &str = "meta";
const CMD: &str = "cmd";

const ALT: &str = "alt";
const CONTROL: &str = "ctrl";
const SHIFT: &str = "shift";

// For compatibility
#[cfg(target_os = "macos")]
const META_CMD: &str = CMD;
#[cfg(not(target_os = "macos"))]
const META_CMD: &str = META;

impl TryFrom<String> for KeyModifiers {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let splits = value.split("+");
        let mut modifiers = KeyModifiers::default();
        for modif in splits {
            match modif {
                CONTROL => modifiers.with_ctrl(),
                SHIFT => modifiers.with_shift(),
                ALT => modifiers.with_alt(),
                META => modifiers.with_meta_cmd(),
                CMD => modifiers.with_meta_cmd(),
                _ => return Err(format!("Non valid modifier value: {}", modif)),
            }
        }
        Ok(modifiers)
    }
}

impl From<KeyModifiers> for String {
    fn from(value: KeyModifiers) -> Self {
        value.to_string()
    }
}

impl From<Modifiers> for KeyModifiers {
    fn from(value: Modifiers) -> Self {
        let mut km = KeyModifiers::default();
        if value.shift() {
            km.with_shift();
        }
        if value.ctrl() {
            km.with_ctrl();
        }
        if value.alt() {
            km.with_alt();
        }
        if value.meta() {
            km.with_meta_cmd();
        }
        km
    }
}

impl Display for KeyModifiers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut modifiers = vec![];
        if self.is_ctrl() {
            modifiers.push(CONTROL);
        }
        if self.is_alt() {
            modifiers.push(ALT);
        }
        if self.is_meta_cmd() {
            modifiers.push(META_CMD);
        }
        if self.is_shift() {
            modifiers.push(SHIFT);
        }
        let string = modifiers.join("+");
        write!(f, "{}", string)
    }
}

impl KeyModifiers {
    pub fn new() -> Self {
        KeyModifiers::default()
    }

    pub fn is_empty(&self) -> bool {
        !(self.alt || self.ctrl || self.cmd || !self.shift)
    }

    pub fn with_shift(&mut self) {
        self.shift = true;
    }
    pub fn with_ctrl(&mut self) {
        self.ctrl = true;
    }
    pub fn with_alt(&mut self) {
        self.alt = true;
    }
    pub fn with_meta_cmd(&mut self) {
        self.cmd = true;
    }

    pub fn and_shift(mut self) -> Self {
        self.with_shift();
        self
    }
    pub fn and_ctrl(mut self) -> Self {
        self.with_ctrl();
        self
    }
    pub fn and_alt(mut self) -> Self {
        self.with_alt();
        self
    }
    pub fn and_meta_cmd(mut self) -> Self {
        self.with_meta_cmd();
        self
    }
    /// Return `true` if a shift key is pressed.
    pub fn is_shift(&self) -> bool {
        self.shift
    }

    /// Return `true` if a control key is pressed.
    pub fn is_ctrl(&self) -> bool {
        self.ctrl
    }

    /// Return `true` if an alt key is pressed.
    pub fn is_alt(&self) -> bool {
        self.alt
    }

    /// Return `true` if a meta key is pressed.
    pub fn is_meta_cmd(&self) -> bool {
        self.cmd
    }
}

#[cfg(test)]
mod tests {
    use crate::utils::keys::{key_combo::KeyCombo, key_strike::KeyStrike};

    use super::KeyModifiers;

    #[test]
    fn serialize_keymodifier() -> anyhow::Result<()> {
        let mut km = KeyModifiers::default();
        km.with_shift();

        let km_ser = km.to_string();
        assert_eq!("shift", km_ser);

        km.with_ctrl();
        let km_ser = km.to_string();
        assert_eq!("ctrl+shift", km_ser);
        Ok(())
    }

    #[test]
    fn deserialize_keymodifier() -> anyhow::Result<()> {
        let text = "meta+shift";
        let km = KeyModifiers::try_from(text.to_string());

        assert!(km.is_ok());

        let km = km.unwrap();
        assert!(km.cmd);
        assert!(km.shift);
        assert!(!km.ctrl);
        assert!(!km.alt);

        Ok(())
    }

    #[test]
    fn serialize_keycombo() {
        let kc = KeyCombo::new(
            KeyModifiers::new().and_meta_cmd().and_ctrl(),
            crate::utils::keys::key_strike::KeyStrike::KeyN,
        );

        let kc_ser = kc.to_string();
        assert_eq!("ctrl+cmd & N", kc_ser);
    }

    #[test]
    fn deserialize_keycombo_meta() {
        let string = "shift+meta & H".to_string();

        let kc = KeyCombo::try_from(string).unwrap();

        assert!(kc.modifiers.shift);
        assert!(kc.modifiers.cmd);
        assert!(!kc.modifiers.ctrl);
        assert!(!kc.modifiers.alt);
        assert_eq!(kc.key, KeyStrike::KeyH);
    }

    #[test]
    fn deserialize_keycombo_cmd() {
        let string = "shift+cmd & H".to_string();

        let kc = KeyCombo::try_from(string).unwrap();

        assert!(kc.modifiers.shift);
        assert!(kc.modifiers.cmd);
        assert!(!kc.modifiers.ctrl);
        assert!(!kc.modifiers.alt);
        assert_eq!(kc.key, KeyStrike::KeyH);
    }

    #[test]
    fn deserialize_keycombo_no_mod() {
        let string = "L".to_string();

        let kc = KeyCombo::try_from(string).unwrap();

        assert!(!kc.modifiers.shift);
        assert!(!kc.modifiers.cmd);
        assert!(!kc.modifiers.ctrl);
        assert!(!kc.modifiers.alt);
        assert_eq!(kc.key, KeyStrike::KeyL);
    }
}
