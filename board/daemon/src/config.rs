use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const DEFAULT_PATH: &str = "/etc/sending-stones/config.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub port: u16,
    pub pad_count: u8,
    /// Optional 4-digit PIN; when set, every new claim and takeover needs it.
    pub pin: Option<u16>,
    /// Defaults to /dev/hidg0 .. /dev/hidg<pad_count-1>.
    pub hid_paths: Vec<PathBuf>,
    pub mock: bool,
    pub failsafe_ms: u64,
    pub reservation_s: u64,
    pub mdns: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            port: proto::frame::DEFAULT_PORT,
            pad_count: 2,
            pin: None,
            hid_paths: Vec::new(),
            mock: false,
            failsafe_ms: 300,
            reservation_s: 120,
            mdns: true,
        }
    }
}

impl Config {
    /// A missing file is fine (all defaults); a malformed one is an error.
    pub fn load(path: &Path) -> Result<Config> {
        let config = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.pad_count == 0 {
            bail!("pad_count must be at least 1");
        }
        if let Some(pin) = self.pin {
            if pin > proto::frame::MAX_PIN {
                bail!("pin must be 4 digits (0000-9999)");
            }
        }
        if !self.hid_paths.is_empty() && self.hid_paths.len() != self.pad_count as usize {
            bail!("hid_paths lists {} paths for {} pads", self.hid_paths.len(), self.pad_count);
        }
        Ok(())
    }

    pub fn hid_path(&self, pad_index: usize) -> PathBuf {
        self.hid_paths
            .get(pad_index)
            .cloned()
            .unwrap_or_else(|| PathBuf::from(format!("/dev/hidg{pad_index}")))
    }

    pub fn failsafe(&self) -> Duration {
        Duration::from_millis(self.failsafe_ms)
    }

    pub fn reservation(&self) -> Duration {
        Duration::from_secs(self.reservation_s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_spec_defaults() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!((c.port, c.pad_count, c.failsafe_ms, c.reservation_s), (40400, 2, 300, 120));
        assert_eq!(c.hid_path(1), PathBuf::from("/dev/hidg1"));
    }

    #[test]
    fn rejects_bad_values() {
        let bad_pin: Config = toml::from_str("pin = 12345").unwrap();
        assert!(bad_pin.validate().is_err());
        let paths: Config = toml::from_str("hid_paths = [\"/dev/hidg0\"]").unwrap();
        assert!(paths.validate().is_err());
        assert!(toml::from_str::<Config>("typo = 1").is_err());
    }
}
