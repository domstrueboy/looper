use looper_hal::{Backends, DeviceInfo, Direction};

use crate::audio::engine;
use crate::config::AppConfig;

/// A backend as the picker shows it: what to store, and what to read.
pub struct BackendChoice {
    pub id: String,
    pub label: String,
}

/// State behind the settings screen: what there is to choose from, and
/// the choice itself. Rendered by the UI crate's settings screen.
pub struct SettingsState {
    pub backends: Vec<BackendChoice>,
    /// Devices of the chosen backend that can capture.
    pub inputs: Vec<String>,
    /// Those that can play. The same list as `inputs` on a backend whose
    /// devices do both.
    pub outputs: Vec<String>,
    /// Whether the chosen input device plays as well as captures. False
    /// means the output has to be picked separately.
    pub duplex: bool,
    /// Rates both ends will take, which is not always either one's own
    /// list: a shared-mode capture endpoint offers only its own mix rate
    /// while its render half claims a range it would have to resample to
    /// reach.
    pub sample_rates: Vec<u32>,
    pub input_channels: u16,
    /// The choice, already in the shape it will be started and saved in.
    /// Held as an `AppConfig` rather than as a field per setting: a
    /// screen-shaped copy has to be mapped back and forth, and every
    /// setting added has to be added to both.
    pub config: AppConfig,
    pub error: Option<String>,
}

impl SettingsState {
    /// The saved choice, with nothing looked up yet.
    ///
    /// Asking the backends is `refresh`, deliberately not done here: the
    /// caller has to be able to stop a running looper before anything
    /// enumerates, and it can only do that once this screen exists to
    /// put in its place.
    pub fn new(error: Option<String>) -> Self {
        Self {
            backends: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            duplex: true,
            sample_rates: Vec::new(),
            input_channels: 0,
            config: AppConfig::load().unwrap_or_default(),
            error,
        }
    }

    /// Asks what there is, and pre-selects whatever is saved, so
    /// reopening settings doesn't reset every list to the top.
    pub fn refresh(&mut self, backends: &Backends) {
        self.backends = backends
            .iter()
            .map(|backend| BackendChoice {
                id: backend.id().as_str().to_string(),
                label: backend.label().to_string(),
            })
            .collect();

        // A saved backend this build doesn't have - an ASIO config
        // carried to a machine with no driver - falls back rather than
        // leaving a name nothing matches.
        if !self.backends.iter().any(|b| b.id == self.config.backend) {
            self.config.backend = self
                .backends
                .first()
                .map(|b| b.id.clone())
                .unwrap_or_default();
        }
        self.refresh_for_selected_backend(backends);
    }

    /// Takes the form's new values, and asks again only for what a
    /// changed choice decides: a backend, its devices; a device, its
    /// rates and channels. Nothing else enumerates - a slider dragged
    /// doesn't load a driver.
    pub fn edit(&mut self, config: AppConfig, backends: &Backends) {
        let backend_changed = config.backend != self.config.backend;
        let device_changed = config.device_name != self.config.device_name
            || config.output_device_name != self.config.output_device_name;
        self.config = config;
        if backend_changed {
            self.refresh_for_selected_backend(backends);
        } else if device_changed {
            self.refresh_for_selected_device(backends);
        }
    }

    /// Device lists are per-backend, so changing backend re-queries them.
    pub fn refresh_for_selected_backend(&mut self, backends: &Backends) {
        let devices = backends
            .by_name(&self.config.backend)
            .and_then(|backend| backend.devices().ok())
            .unwrap_or_default();

        self.inputs = names(&devices, Direction::Input);
        self.outputs = names(&devices, Direction::Output);

        if !self.inputs.contains(&self.config.device_name) {
            self.config.device_name = self.inputs.first().cloned().unwrap_or_default();
        }
        self.refresh_for_selected_device(backends);
    }

    /// Rates and channels are per-device, so a device change re-queries
    /// them and drops any selection the new device doesn't offer. A
    /// selection it does offer is kept - switching devices shouldn't
    /// silently move a rate that both of them support.
    pub fn refresh_for_selected_device(&mut self, backends: &Backends) {
        let input = engine::find_device(
            backends,
            &self.config.backend,
            &self.config.device_name,
            Direction::Input,
        )
        .ok();
        self.duplex = input
            .as_ref()
            .is_some_and(|device| device.direction == Direction::Duplex);

        // One device does both, so there is nothing to choose and
        // nothing to store.
        if self.duplex {
            self.config.output_device_name = None;
        } else if !self
            .outputs
            .contains(&self.config.output_device().to_string())
        {
            self.config.output_device_name = self.outputs.first().cloned();
        }

        let output = engine::find_device(
            backends,
            &self.config.backend,
            self.config.output_device(),
            Direction::Output,
        )
        .ok();

        // Falls back to empty rather than surfacing the error: a device
        // that answers neither can't be started, and the screen says so
        // from the empty lists themselves.
        let caps = |device: &Option<DeviceInfo>| {
            device
                .as_ref()
                .and_then(|device| backends.get(device.id.backend)?.caps(device).ok())
        };
        let input_caps = caps(&input);
        let output_caps = caps(&output);

        self.input_channels = input_caps.as_ref().map(|c| c.input_channels).unwrap_or(0);
        self.sample_rates = match (&input_caps, &output_caps) {
            (Some(input), Some(output)) => input
                .sample_rates
                .iter()
                .copied()
                .filter(|rate| output.sample_rates.contains(rate))
                .collect(),
            _ => Vec::new(),
        };

        if !self.sample_rates.contains(&self.config.sample_rate) {
            self.config.sample_rate = self.sample_rates.first().copied().unwrap_or_default();
        }
        if self.config.input_channel >= self.input_channels {
            self.config.input_channel = 0;
        }
    }
}

fn names(devices: &[DeviceInfo], wanted: Direction) -> Vec<String> {
    devices
        .iter()
        .filter(|device| match wanted {
            Direction::Output => device.direction.can_play(),
            _ => device.direction.can_capture(),
        })
        .map(|device| device.id.name.clone())
        .collect()
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
