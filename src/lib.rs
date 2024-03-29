#![cfg_attr(docsrs, feature(doc_cfg))]

//! # Crazyradio driver for Rust
//!
//! This crate aims at providing a Rust API for the [Crazyradio](https://www.bitcraze.io/products/crazyradio-pa/)
//! USB Dongle.
//!
//! Available Cargo features:
//!  - **shared_radio** enables [SharedCrazyradio] object that allows to share a radio between threads
//!  - **async** enables async function to create a [Crazyradio] object and use the [SharedCrazyradio]
//!  - **serde** emables [serde](https://crates.io/crates/serde) serialization/deserialization of the [Channel] struct

#[cfg(feature = "shared_radio")]
mod shared_radio;
#[cfg(feature = "shared_radio")]
pub use crate::shared_radio::SharedCrazyradio;

use core::time::Duration;
#[cfg(feature = "serde_support")]
use serde::{Deserialize, Serialize};

type Result<T> = std::result::Result<T, Error>;

fn find_crazyradio(
    nth: Option<usize>,
    serial: Option<&str>,
) -> Result<rusb::Device<rusb::GlobalContext>> {
    let mut n = 0;

    for device in rusb::devices()?.iter() {
        let device_desc = device.device_descriptor()?;

        if device_desc.vendor_id() == 0x1915 && device_desc.product_id() == 0x7777 {
            let handle = device.open()?;

            if (nth == None || nth == Some(n))
                && (serial == None || serial == Some(&get_serial(&device_desc, &handle)?))
            {
                return Ok(device);
            }
            n += 1;
        }
    }
    Err(Error::NotFound)
}

fn get_serial<T: rusb::UsbContext>(
    device_desc: &rusb::DeviceDescriptor,
    handle: &rusb::DeviceHandle<T>,
) -> Result<String> {
    let languages = handle.read_languages(Duration::from_secs(1))?;

    if !languages.is_empty() {
        let serial =
            handle.read_serial_number_string(languages[0], device_desc, Duration::from_secs(1))?;
        Ok(serial)
    } else {
        Err(Error::NotFound)
    }
}

fn list_crazyradio_serials() -> Result<Vec<String>> {
    let mut serials = vec![];

    for device in rusb::devices()?.iter() {
        let device_desc = device.device_descriptor()?;

        if device_desc.vendor_id() == 0x1915 && device_desc.product_id() == 0x7777 {
            let handle: rusb::DeviceHandle<rusb::GlobalContext> = device.open()?;

            let languages = handle.read_languages(Duration::from_secs(1))?;

            if !languages.is_empty() {
                let serial = handle.read_serial_number_string(
                    languages[0],
                    &device_desc,
                    Duration::from_secs(1),
                )?;
                serials.push(serial);
            }
        }
    }
    Ok(serials)
}

enum UsbCommand {
    SetRadioChannel = 0x01,
    SetRadioAddress = 0x02,
    SetDataRate = 0x03,
    SetRadioPower = 0x04,
    SetRadioArd = 0x05,
    SetRadioArc = 0x06,
    AckEnable = 0x10,
    SetContCarrier = 0x20,
    // ScanChannels = 0x21,
    LaunchBootloader = 0xff,
}

/// Represents a Crazyradio
///
/// Holds the USB connection to a Crazyradio dongle.
/// The connection is closed when this object goes out of scope.Crazyradio
///
/// Usage example:
/// ```no_run
/// use crazyradio::{Crazyradio, Error, Channel};
///
/// fn main() -> Result<(), Error> {
///     let mut cr = Crazyradio::open_first()?;   // Open the first detected dongle
///
///     // Set the radio channel
///     cr.set_channel(Channel::from_number(42).unwrap());
///
///     // Send a `null` packet
///     let mut ack_data = [0u8; 32];
///     let ack = cr.send_packet(&[0xff], &mut ack_data)?;
///
///     println!("Ack received: {}, length: {}, data: {:?}", ack.received,
///                                                          ack.length,
///                                                          &ack_data[..ack.length]);
///
///     Ok(())
/// }
/// ```
pub struct Crazyradio {
    device_desciptor: rusb::DeviceDescriptor,
    device_handle: rusb::DeviceHandle<rusb::GlobalContext>,

    cache_settings: bool,

    // Settings cache
    channel: Channel,
    address: [u8; 5],
    datarate: Datarate,
}

impl Crazyradio {
    /// Open the first Crazyradio detected and returns a Crazyradio object.
    ///
    /// The dongle is reset to boot values before being returned
    pub fn open_first() -> Result<Self> {
        Crazyradio::open_nth(0)
    }

    /// Open the nth Crazyradio detected and returns a Crazyradio object.
    ///
    /// Radios are ordered appearance in the USB device list. This order is
    /// platform-specific.
    ///
    /// The dongle is reset to boot values before being returned
    pub fn open_nth(nth: usize) -> Result<Self> {
        Self::open_generic(Some(nth), None)
    }

    /// Open a Crazyradio by specifying its serial number
    ///
    /// Example:
    /// ```no_run
    /// use crazyradio::Crazyradio;
    /// # fn main() -> Result<(), crazyradio::Error> {
    /// let mut cr = Crazyradio::open_by_serial("FD61E54B7A")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn open_by_serial(serial: &str) -> Result<Self> {
        Self::open_generic(None, Some(serial))
    }

    // Generic version of the open function, called by the other open_* functions
    fn open_generic(nth: Option<usize>, serial: Option<&str>) -> Result<Self> {
        let device = find_crazyradio(nth, serial)?;

        let device_desciptor = device.device_descriptor()?;
        let mut device_handle = device.open()?;

        device_handle.claim_interface(0)?;

        // Make sure the dongle version is >= 0.5
        let version = device_desciptor.device_version();
        let version = version.major() as f64
            + (version.minor() as f64 / 10.0)
            + (version.sub_minor() as f64 / 100.0);
        if version < 0.5 {
            return Err(Error::DongleVersionNotSupported);
        }

        let mut cr = Crazyradio {
            device_desciptor,
            device_handle,

            cache_settings: true,

            channel: Channel::from_number(2).unwrap(),
            address: [0xe7; 5],
            datarate: Datarate::Dr2M,
        };

        cr.reset()?;

        Ok(cr)
    }

    /// Return an ordered list of serial numbers of connected Crazyradios
    ///
    /// The order of the list is the same as accepted by the open_nth() function.
    pub fn list_serials() -> Result<Vec<String>> {
        list_crazyradio_serials()
    }

    /// Return the serial number of this radio
    pub fn serial(&self) -> Result<String> {
        get_serial(&self.device_desciptor, &self.device_handle)
    }

    /// Reset dongle parameters to boot values.
    ///
    /// This function is called by Crazyradio::open_*.
    pub fn reset(&mut self) -> Result<()> {
        let prev_cache_settings = self.cache_settings;
        self.cache_settings = false;

        self.set_datarate(Datarate::Dr2M)?;
        self.set_channel(Channel::from_number(2).unwrap())?;
        self.set_cont_carrier(false)?;
        self.set_address(&[0xe7, 0xe7, 0xe7, 0xe7, 0xe7])?;
        self.set_power(Power::P0dBm)?;
        self.set_arc(3)?;
        self.set_ard_bytes(32)?;
        self.set_ack_enable(true)?;

        self.cache_settings = prev_cache_settings;

        Ok(())
    }

    /// Enable or disable caching of settings
    ///
    /// If enabled, setting the radio channel, address or datarate will be
    /// ignored if the settings is the same as the one already set in the dongle
    ///
    /// This is enabled by default and is a useful functionality to efficiently
    /// implement communication to multiple device as changing these settings
    /// require USB communication and is quite slow.
    pub fn set_cache_settings(&mut self, cache_settings: bool) {
        self.cache_settings = cache_settings;
    }

    /// Set the radio channel.
    pub fn set_channel(&mut self, channel: Channel) -> Result<()> {
        if !self.cache_settings || self.channel != channel {
            self.device_handle.write_control(
                0x40,
                UsbCommand::SetRadioChannel as u8,
                channel.0 as u16,
                0,
                &[],
                Duration::from_secs(1),
            )?;
            self.channel = channel;
        }

        Ok(())
    }

    /// Set the datarate.
    pub fn set_datarate(&mut self, datarate: Datarate) -> Result<()> {
        if !self.cache_settings || self.datarate != datarate {
            self.device_handle.write_control(
                0x40,
                UsbCommand::SetDataRate as u8,
                datarate as u16,
                0,
                &[],
                Duration::from_secs(1),
            )?;
            self.datarate = datarate;
        }

        Ok(())
    }

    /// Set the radio address.
    pub fn set_address(&mut self, address: &[u8; 5]) -> Result<()> {
        if !self.cache_settings || self.address != *address {
            self.device_handle.write_control(
                0x40,
                UsbCommand::SetRadioAddress as u8,
                0,
                0,
                address,
                Duration::from_secs(1),
            )?;
            self.address.copy_from_slice(address);
        }

        Ok(())
    }

    /// Set the transmit power.
    pub fn set_power(&mut self, power: Power) -> Result<()> {
        self.device_handle.write_control(
            0x40,
            UsbCommand::SetRadioPower as u8,
            power as u16,
            0,
            &[],
            Duration::from_secs(1),
        )?;
        Ok(())
    }

    /// Set time to wait for the ack packet.
    pub fn set_ard_time(&mut self, delay: Duration) -> Result<()> {
        if delay <= Duration::from_millis(4000) {
            // Set to step above or equal to `delay`
            let ard = (delay.as_millis() as u16 / 250) - 1;
            self.device_handle.write_control(
                0x40,
                UsbCommand::SetRadioArd as u8,
                ard,
                0,
                &[],
                Duration::from_secs(1),
            )?;
            Ok(())
        } else {
            Err(Error::InvalidArgument)
        }
    }

    /// Set time to wait for the ack packet by specifying the max byte-length of the ack payload.
    pub fn set_ard_bytes(&mut self, nbytes: u8) -> Result<()> {
        if nbytes <= 32 {
            self.device_handle.write_control(
                0x40,
                UsbCommand::SetRadioArd as u8,
                0x80 | nbytes as u16,
                0,
                &[],
                Duration::from_secs(1),
            )?;
            Ok(())
        } else {
            Err(Error::InvalidArgument)
        }
    }

    /// Set the number of time the radio will retry to send the packet if an ack packet is not received in time.
    pub fn set_arc(&mut self, arc: usize) -> Result<()> {
        if arc <= 15 {
            self.device_handle.write_control(
                0x40,
                UsbCommand::SetRadioArc as u8,
                arc as u16,
                0,
                &[],
                Duration::from_secs(1),
            )?;
            Ok(())
        } else {
            Err(Error::InvalidArgument)
        }
    }

    /// Set if the radio waits for an ack packet.
    ///
    /// Should be disabled when sending broadcast packets.
    pub fn set_ack_enable(&mut self, ack_enable: bool) -> Result<()> {
        self.device_handle.write_control(
            0x40,
            UsbCommand::AckEnable as u8,
            ack_enable as u16,
            0,
            &[],
            Duration::from_secs(1),
        )?;
        Ok(())
    }

    /// Sends a packet to a range of channel and returns a list of channel that acked
    ///
    /// Used to activally scann for receives on channels. This function sends
    pub fn scan_channels(
        &mut self,
        start: Channel,
        stop: Channel,
        packet: &[u8],
    ) -> Result<Vec<Channel>> {
        let mut ack_data = [0u8; 32];
        let mut result: Vec<Channel> = vec![];
        for ch in start.0..stop.0 + 1 {
            let channel = Channel::from_number(ch).unwrap();
            self.set_channel(channel)?;
            let ack = self.send_packet(packet, &mut ack_data)?;
            if ack.received {
                result.push(channel);
            }
        }
        Ok(result)
    }

    /// Launch the bootloader.
    ///
    /// Consumes the Crazyradio since it is not usable after that (it is in bootlaoder mode ...).
    pub fn launch_bootloader(self) -> Result<()> {
        self.device_handle.write_control(
            0x40,
            UsbCommand::LaunchBootloader as u8,
            0,
            0,
            &[],
            Duration::from_secs(1),
        )?;
        Ok(())
    }

    /// Set the radio in continious carrier mode.
    ///
    /// In continious carrier mode, the radio will transmit a continious sine
    /// wave at the setup channel frequency using the setup transmit power.
    pub fn set_cont_carrier(&mut self, enable: bool) -> Result<()> {
        self.device_handle.write_control(
            0x40,
            UsbCommand::SetContCarrier as u8,
            enable as u16,
            0,
            &[],
            Duration::from_secs(1),
        )?;
        Ok(())
    }

    /// Send a data packet and receive an ack packet.
    ///
    /// # Arguments
    ///
    ///  * `data`: Up to 32 bytes of data to be send.
    ///  * `ack_data`: Buffer to hold the data received from the ack packet
    ///                payload. The ack payload can be up to 32 bytes, if this
    ///                buffer length is lower than 32 bytes the ack data might
    ///                be truncated. The length of the ack payload is returned
    ///                in Ack::length.
    pub fn send_packet(&mut self, data: &[u8], ack_data: &mut [u8]) -> Result<Ack> {
        self.device_handle
            .write_bulk(0x01, data, Duration::from_secs(1))?;
        let mut received_data = [0u8; 33];
        let received =
            self.device_handle
                .read_bulk(0x81, &mut received_data, Duration::from_secs(1))?;

        if ack_data.len() <= 32 {
            ack_data.copy_from_slice(&received_data[1..ack_data.len() + 1]);
        } else {
            ack_data
                .split_at_mut(32)
                .0
                .copy_from_slice(&received_data[1..33]);
        }

        Ok(Ack {
            received: received_data[0] & 0x01 != 0,
            power_detector: received_data[0] & 0x02 != 0,
            retry: ((received_data[0] & 0xf0) >> 4) as usize,
            length: received - 1,
        })
    }
}

/// # Async implementations
///
/// Async version of open/getserial functions.
///
/// Implemented by launching a thread, calling the sync function and passing the
/// result back though a channel.
/// This is not the most efficient implementation but it keeps the lib executor-independent
/// and these functions are only one-time-call in most programs.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
impl Crazyradio {

    /// Async vesion of [Crazyradio::open_first()]
    pub async fn open_first_async() -> Result<Self> {
        let (tx, rx) = flume::bounded(0);

        std::thread::spawn(move || tx.send(Self::open_first()));

        rx.recv_async().await.unwrap()
    }

    /// Async vesion of [Crazyradio::open_nth()]
    pub async fn open_nth_async(nth: usize) -> Result<Self> {
        let (tx, rx) = flume::bounded(0);

        std::thread::spawn(move || tx.send(Self::open_nth(nth)));

        rx.recv_async().await.unwrap()
    }

    /// Async vesion of [Crazyradio::open_by_serial()]
    pub async fn open_by_serial_async(serial: &str) -> Result<Self> {
        let serial = serial.to_owned();

        let (tx, rx) = flume::bounded(0);

        std::thread::spawn(move || tx.send(Self::open_by_serial(&serial)));

        rx.recv_async().await.unwrap()
    }

    /// Async vesion of [Crazyradio::list_serials()]
    pub async fn list_serials_async() -> Result<Vec<String>> {
        let (tx, rx) = flume::bounded(0);

        std::thread::spawn(move || tx.send(Self::list_serials()));

        rx.recv_async().await.unwrap()
    }
}

#[derive(thiserror::Error, Debug, Clone)]
pub enum Error {
    #[error("Usb Error: {0}:?")]
    UsbError(rusb::Error),
    #[error("Crazyradio not found")]
    NotFound,
    #[error("Invalid arguments")]
    InvalidArgument,
    #[error("Crazyradio version not supported")]
    DongleVersionNotSupported,
}

impl From<rusb::Error> for Error {
    fn from(usb_error: rusb::Error) -> Self {
        Error::UsbError(usb_error)
    }
}

/// Ack status of a sent packet
#[derive(Debug, Copy, Clone)]
pub struct Ack {
    /// At true if an ack packet has been received
    pub received: bool,
    /// Value of the nRF24 power detector when receiving the ack packet
    pub power_detector: bool,
    /// Number of time the packet was sent before an ack was received
    pub retry: usize,
    /// Length of the ack payload
    pub length: usize,
}

/// Radio channel
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[cfg_attr(feature = "serde_support", derive(Serialize))]
pub struct Channel(u8);

#[cfg(feature = "serde_support")]
impl<'de> Deserialize<'de> for Channel {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Channel, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let ch_number: u8 = Deserialize::deserialize(deserializer)?;
        let channel = Channel::from_number(ch_number)
            .map_err(|e| serde::de::Error::custom(format!("{:?}", e)))?;
        Ok(channel)
    }
}

impl Channel {
    pub fn from_number(channel: u8) -> Result<Self> {
        if channel < 126 {
            Ok(Channel(channel))
        } else {
            Err(Error::InvalidArgument)
        }
    }
}

impl From<Channel> for u8 {
    fn from(val: Channel) -> Self {
        val.0
    }
}

/// Radio datarate
#[derive(Copy, Clone, PartialEq)]
pub enum Datarate {
    Dr250K = 0,
    Dr1M = 1,
    Dr2M = 2,
}

/// Radio power
pub enum Power {
    Pm18dBm = 0,
    Pm12dBm = 1,
    Pm6dBm = 2,
    P0dBm = 3,
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "serde_support")]
    use serde_json;

    #[cfg(feature = "serde_support")]
    use super::Channel;

    #[test]
    #[cfg(feature = "serde_support")]
    fn test_that_deserializing_a_correct_channel_works() {
        let test_str = "42";

        let result: Result<Channel, serde_json::Error> = serde_json::from_str(test_str);

        assert!(matches!(result, Ok(Channel(42))));
    }

    #[test]
    #[cfg(feature = "serde_support")]
    fn test_that_deserializing_an_incorrect_channel_works() {
        let test_str = "126";

        let result: Result<Channel, serde_json::Error> = serde_json::from_str(test_str);

        assert!(matches!(result, Err(_)));
    }

    #[test]
    #[cfg(feature = "serde_support")]
    fn test_that_serialize_channel_works() {
        let test_channel = Channel(42);

        let result = serde_json::to_string(&test_channel);

        assert!(matches!(result, Ok(str) if str == "42"));
    }
}
