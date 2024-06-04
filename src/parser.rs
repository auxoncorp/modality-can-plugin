use crate::{dbc::EmptyStringExt, CommonConfig};
use auxon_sdk::api::{AttrKey, AttrVal};
use bitvec::prelude::*;
use can_dbc::{
    ByteOrder, Message, MessageId, MultiplexIndicator, Signal, SignalExtendedValueType,
    Transmitter, ValueDescription, ValueType, DBC,
};
use socketcan::{CanAnyFrame, EmbeddedFrame, Id};
use std::collections::HashMap;
use tracing::warn;

#[derive(Debug)]
pub struct ParsedCanFrame {
    id: CanId,
    pub msg_name: Option<String>,
    pub transmitter_node: Option<String>,
    pub attrs: HashMap<AttrKey, AttrVal>,
}

impl ParsedCanFrame {
    pub fn event_name(&self) -> String {
        if let Some(msg) = &self.msg_name {
            msg.to_owned()
        } else {
            format!("{}", self.id)
        }
    }
}

#[derive(Debug)]
pub struct CanParser {
    use_msg_as_event_name: bool,
    id_to_msg_info: HashMap<CanId, DbcMessageInfo>,
}

impl CanParser {
    pub fn new(cfg: &CommonConfig, dbc: Option<&DBC>) -> Result<Self, anyhow::Error> {
        let mut id_to_msg_info = HashMap::new();

        if let Some(dbc) = dbc {
            for msg in dbc.messages().iter() {
                let mut signal_to_type = HashMap::new();
                let mut signal_to_values = HashMap::new();
                let mut muxed_to_info = HashMap::new();

                // Default signal value types, may get overridden by the extended types
                for s in msg.signals().iter() {
                    let typ = match s.value_type() {
                        ValueType::Signed => SignalValueType::Signed,
                        ValueType::Unsigned => SignalValueType::Unsigned,
                    };
                    signal_to_type.insert(s.name().clone(), typ);
                }

                // Apply extended signal types
                for s in dbc
                    .signal_extended_value_type_list()
                    .iter()
                    .filter(|s| *s.message_id() == *msg.message_id())
                {
                    match s.signal_extended_value_type() {
                        SignalExtendedValueType::IEEEfloat32Bit => {
                            signal_to_type.insert(s.signal_name().clone(), SignalValueType::F32);
                        }
                        SignalExtendedValueType::IEEEdouble64bit => {
                            signal_to_type.insert(s.signal_name().clone(), SignalValueType::F64);
                        }
                        SignalExtendedValueType::SignedOrUnsignedInteger => (),
                    };
                }

                // Convert the value descriptions to integer keys
                for vd in dbc.value_descriptions().iter() {
                    if let ValueDescription::Signal {
                        message_id,
                        signal_name,
                        value_descriptions,
                    } = vd
                    {
                        if *message_id == *msg.message_id() {
                            let val_desc_for_sig: &mut ValueDescriptionMap =
                                signal_to_values.entry(signal_name.clone()).or_default();
                            for d in value_descriptions.iter() {
                                // These should always be integers
                                val_desc_for_sig.insert(*d.a() as i64, d.b().clone());
                            }
                        }
                    }
                }

                // Setup the multiplexed signal maps
                if let Some(muxer_sig) = dbc
                    .message_multiplexor_switch(*msg.message_id())
                    .ok()
                    .flatten()
                {
                    // We have a "simple" multiplexed signal (single multiplexor)
                    // Find all of the child signals in the signal tree
                    for s in msg.signals().iter() {
                        let indicator = match s.multiplexer_indicator() {
                            MultiplexIndicator::MultiplexedSignal(i) => *i,
                            MultiplexIndicator::MultiplexorAndMultiplexedSignal(i) => *i,
                            _ => continue,
                        };
                        muxed_to_info.insert(
                            s.name().clone(),
                            MuxedSignalInfo {
                                muxer: muxer_sig.name().clone(),
                                indicator,
                            },
                        );
                    }
                }
                // TODO add support for extended signal multiplexing

                let msg_info = DbcMessageInfo {
                    msg: msg.clone(),
                    signal_state: SignalState {
                        signal_to_type,
                        signal_to_values,
                        muxed_to_info,
                        muxer_to_value: Default::default(),
                    },
                };
                if id_to_msg_info
                    .insert(msg.message_id().raw_can_id(), msg_info)
                    .is_some()
                {
                    warn!(
                        id = ?msg.message_id(),
                        msg = msg.message_name(),
                        "DBC file contains a duplicate message"
                    );
                }
            }
        }

        Ok(Self {
            use_msg_as_event_name: cfg.event_from_message.unwrap_or(true),
            id_to_msg_info,
        })
    }

    pub fn parse(&mut self, frame: &CanAnyFrame) -> Result<ParsedCanFrame, anyhow::Error> {
        // Adds the frame-level info
        let mut pcf = ParsedCanFrame::new(frame);

        // Add DBC-related info
        if let Some(msg_info) = self.id_to_msg_info.get_mut(&pcf.id) {
            if self.use_msg_as_event_name {
                if let Some(msg_name) = msg_info.msg.message_name().empty_opt() {
                    pcf.msg_name = Some(msg_name.to_owned());
                }
            }

            // Message-level info
            pcf.add_dbc_msg_attrs(&msg_info.msg);

            let data = match frame {
                CanAnyFrame::Normal(f) => f.data(),
                CanAnyFrame::Remote(f) => f.data(),
                CanAnyFrame::Error(f) => f.data(),
                CanAnyFrame::Fd(f) => f.data(),
            };

            // Parse the message signal
            if data.len() as u64 == *msg_info.msg.message_size() {
                for signal in msg_info.msg.signals().iter() {
                    if !msg_info
                        .signal_state
                        .signal_to_type
                        .contains_key(signal.name())
                    {
                        warn!(
                            id = ?msg_info.msg.message_id(),
                            msg = msg_info.msg.message_name(),
                            signal = signal.name(),
                            "CAN signal missing value type"
                        );
                    }

                    pcf.add_dbc_signal_attrs(&mut msg_info.signal_state, signal, data);
                }
            } else {
                warn!(
                    id = ?msg_info.msg.message_id(),
                    msg = msg_info.msg.message_name(),
                    data_len = data.len(),
                    msg_size = *msg_info.msg.message_size(),
                    "CAN frame data length doesn't match the message defintion"
                );
            }

            // Clear out any muxer signal state
            msg_info.signal_state.muxer_to_value.clear();
        }

        Ok(pcf)
    }
}

impl ParsedCanFrame {
    fn new(frame: &CanAnyFrame) -> Self {
        let is_extended;
        let is_remote;
        let is_error;
        let is_brs; // FD: bit rate switch
        let is_esi; // FD: error state indicator
        let dlc;
        let id;

        match frame {
            CanAnyFrame::Normal(f) => {
                is_extended = f.is_extended();
                is_remote = f.is_remote_frame();
                is_error = false;
                is_brs = false;
                is_esi = false;
                dlc = f.dlc();
                id = f.id();
            }
            CanAnyFrame::Remote(f) => {
                is_extended = f.is_extended();
                is_remote = f.is_remote_frame();
                is_error = false;
                is_brs = false;
                is_esi = false;
                dlc = f.dlc();
                id = f.id();
            }
            CanAnyFrame::Error(f) => {
                is_extended = f.is_extended();
                is_remote = f.is_remote_frame();
                is_error = true;
                is_brs = false;
                is_esi = false;
                dlc = f.dlc();
                id = f.id();
            }
            CanAnyFrame::Fd(f) => {
                is_extended = f.is_extended();
                is_remote = f.is_remote_frame();
                is_error = false;
                is_brs = f.is_brs();
                is_esi = f.is_esi();
                dlc = f.dlc();
                id = f.id();
            }
        }

        let mut pcf = Self {
            id: id.raw_can_id(),
            msg_name: None,
            transmitter_node: None,
            attrs: Default::default(),
        };

        pcf.add_attr("frame.id", id.raw_can_id());
        pcf.add_attr("frame.dlc", dlc as u64);

        if is_extended {
            pcf.add_attr("frame.extended", true);
        }
        if is_remote {
            pcf.add_attr("frame.remote", true);
        }
        if is_error {
            pcf.add_attr("frame.error", true);
        }
        if is_brs {
            pcf.add_attr("frame.brs", true);
        }
        if is_esi {
            pcf.add_attr("frame.esi", true);
        }

        pcf
    }

    fn add_dbc_msg_attrs(&mut self, msg: &Message) {
        self.add_internal_attr("message.signal.count", msg.signals().len() as u32);
        self.add_attr("message.size", *msg.message_size());
        self.add_attr("message.name", msg.message_name());
        if let Transmitter::NodeName(node) = msg.transmitter() {
            self.add_attr("message.trasmitter", node);
            self.transmitter_node = Some(node.clone());
        }
    }

    fn add_dbc_signal_attrs(
        &mut self,
        signal_state: &mut SignalState,
        signal: &Signal,
        data: &[u8],
    ) {
        // Skip if this is a multiplexed signal and the muxer indicator doesn't match
        let is_muxed = matches!(
            signal.multiplexer_indicator(),
            MultiplexIndicator::MultiplexedSignal(_)
                | MultiplexIndicator::MultiplexorAndMultiplexedSignal(_)
        );
        if is_muxed {
            if let Some(muxed_info) = signal_state.muxed_to_info.get(signal.name()) {
                if let Some(muxer_indicator) = signal_state.muxer_to_value.get(&muxed_info.muxer) {
                    if *muxer_indicator != muxed_info.indicator {
                        return;
                    }
                }
            }
        }

        if let Some(val) = parse_signal(signal_state, signal, data) {
            // I think the spec prohibits this...
            let normalized_signal_name = signal.name().replace(' ', "_");
            if let Some(unit) = signal.unit().empty_opt() {
                self.add_attr(format!("{normalized_signal_name}.unit"), unit);
            }
            self.add_attr(normalized_signal_name, val);
        } else {
            warn!(signal = signal.name(), "Failed to parse signal");
        }
    }

    fn add_attr<K: AsRef<str>, V: Into<AttrVal>>(&mut self, k: K, v: V) {
        let k = format!("event.{}", k.as_ref());
        self.attrs.insert(k.into(), v.into());
    }

    fn add_internal_attr<K: AsRef<str>, V: Into<AttrVal>>(&mut self, k: K, v: V) {
        let k = format!("event.internal.modality_can.{}", k.as_ref());
        self.attrs.insert(k.into(), v.into());
    }
}

type CanId = u32;

trait RawCanIdExt {
    /// Does *not* contain the extended bit for extended IDs
    fn raw_can_id(&self) -> CanId;
}

impl RawCanIdExt for MessageId {
    fn raw_can_id(&self) -> CanId {
        match self {
            MessageId::Standard(id) => *id as CanId,
            MessageId::Extended(id) => *id,
        }
    }
}

impl RawCanIdExt for Id {
    fn raw_can_id(&self) -> CanId {
        match self {
            Id::Standard(id) => id.as_raw() as CanId,
            Id::Extended(id) => id.as_raw(),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum SignalValueType {
    Signed,
    Unsigned,
    F32,
    F64,
}

#[derive(Debug)]
struct DbcMessageInfo {
    msg: Message,
    signal_state: SignalState,
}

#[derive(Debug)]
struct SignalState {
    signal_to_type: HashMap<SignalName, SignalValueType>,
    signal_to_values: HashMap<SignalName, ValueDescriptionMap>,
    muxed_to_info: HashMap<MuxedSignal, MuxedSignalInfo>,
    /// Set when a multiplexor signal is read, contains it's value.
    /// Cleared after processing each frame.
    muxer_to_value: HashMap<MuxerSignal, MuxerIndicatorValue>,
}

type SignalName = String;
type ValueDescriptionMap = HashMap<i64, String>;

type MuxerSignal = String;
type MuxerIndicatorValue = u64;

type MuxedSignal = String;

#[derive(Debug, PartialEq, Hash)]
struct MuxedSignalInfo {
    /// The parent multiplexor indicator signal
    muxer: MuxerSignal,
    /// The indicator value of the parent multiplexor
    /// that flags this multiplexed signal as active.
    indicator: MuxerIndicatorValue,
}

fn parse_signal(signal_state: &mut SignalState, sig: &Signal, data: &[u8]) -> Option<AttrVal> {
    let typ = *signal_state.signal_to_type.get(sig.name())?;

    let mut raw = parse_raw_val(sig, typ, data)?;

    let maybe_muxer_value = if *sig.multiplexer_indicator() == MultiplexIndicator::Multiplexor {
        if let RawVal::U64(indicator) = raw {
            Some(indicator)
        } else {
            warn!(
                signal = sig.name(),
                "Multiplexor signal is expected to be an unsigned type"
            );
            None
        }
    } else {
        None
    };

    let maybe_value_description = signal_state
        .signal_to_values
        .get(sig.name())
        .and_then(|vd| match raw {
            RawVal::I64(v) => vd.get(&v),
            RawVal::U64(v) => vd.get(&(v as i64)),
            _ => None,
        });

    if let Some(muxer_value) = maybe_muxer_value {
        signal_state
            .muxer_to_value
            .insert(sig.name().clone(), muxer_value);
        Some(muxer_value.into())
    } else if let Some(val_desc) = maybe_value_description {
        Some(val_desc.as_str().into())
    } else if sig.signal_size == 1 {
        raw.as_bool()
    } else if is_float(sig) || typ == SignalValueType::F32 || typ == SignalValueType::F64 {
        // Scaling/offset floats always promote value to f64
        raw.promote_to_f64();
        raw.as_scaled_float(sig.factor, sig.offset, sig.min, sig.max)
    } else {
        raw.as_scaled_int(sig.factor, sig.offset, sig.min, sig.max)
    }
}

fn parse_raw_val(sig: &Signal, typ: SignalValueType, data: &[u8]) -> Option<RawVal> {
    let (bit_start, bit_end) = signal_start_end_bit(sig, data.len())?;
    let raw = if sig.byte_order() == &ByteOrder::LittleEndian {
        let bits = data.try_view_bits::<Lsb0>().ok()?;
        match typ {
            SignalValueType::Signed => RawVal::I64(bits[bit_start..bit_end].load_le::<i64>()),
            SignalValueType::Unsigned => RawVal::U64(bits[bit_start..bit_end].load_le::<u64>()),
            SignalValueType::F32 => {
                RawVal::F32(f32::from_bits(bits[bit_start..bit_end].load_le::<u32>()))
            }
            SignalValueType::F64 => {
                RawVal::F64(f64::from_bits(bits[bit_start..bit_end].load_le::<u64>()))
            }
        }
    } else {
        let bits = data.try_view_bits::<Msb0>().ok()?;
        match typ {
            SignalValueType::Signed => RawVal::I64(bits[bit_start..bit_end].load_be::<i64>()),
            SignalValueType::Unsigned => RawVal::U64(bits[bit_start..bit_end].load_be::<u64>()),
            SignalValueType::F32 => {
                RawVal::F32(f32::from_bits(bits[bit_start..bit_end].load_be::<u32>()))
            }
            SignalValueType::F64 => {
                RawVal::F64(f64::from_bits(bits[bit_start..bit_end].load_be::<u64>()))
            }
        }
    };
    Some(raw)
}

#[derive(Debug)]
enum RawVal {
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
}

impl RawVal {
    fn as_bool(&self) -> Option<AttrVal> {
        Some(match self {
            RawVal::I64(v) => (*v != 0).into(),
            RawVal::U64(v) => (*v != 0).into(),
            _ => return None,
        })
    }

    fn promote_to_f64(&mut self) {
        let f = match self {
            RawVal::I64(v) => *v as f64,
            RawVal::U64(v) => *v as f64,
            RawVal::F32(v) => *v as f64,
            RawVal::F64(v) => *v,
        };
        *self = RawVal::F64(f);
    }

    fn as_scaled_float(&self, scale: f64, offset: f64, min: f64, max: f64) -> Option<AttrVal> {
        let f = match self {
            RawVal::F32(v) => *v as f64,
            RawVal::F64(v) => *v,
            _ => return None,
        };
        let sf = (f * scale) + offset;
        if min == 0.0 && max == 0.0 {
            Some(AttrVal::from(sf))
        } else {
            Some(AttrVal::from(sf.clamp(min, max)))
        }
    }

    fn as_scaled_int(&self, scale: f64, offset: f64, min: f64, max: f64) -> Option<AttrVal> {
        match self {
            RawVal::I64(v) => {
                let s = scale as i64;
                let o = offset as i64;
                let min = min as i64;
                let max = max as i64;
                v.checked_mul(s).and_then(|n| n.checked_add(o)).map(|n| {
                    if min == 0 && max == 0 {
                        n.into()
                    } else {
                        n.clamp(min, max).into()
                    }
                })
            }
            RawVal::U64(v) => {
                let s = scale as u64;
                let o = offset as u64;
                let min = min as u64;
                let max = max as u64;
                v.checked_mul(s).and_then(|n| n.checked_add(o)).map(|n| {
                    if min == 0 && max == 0 {
                        n.into()
                    } else {
                        n.clamp(min, max).into()
                    }
                })
            }
            _ => None,
        }
    }
}

fn is_float(sig: &Signal) -> bool {
    sig.offset.fract() != 0.0 || sig.factor.fract() != 0.0
}

fn signal_start_end_bit(sig: &Signal, msg_size_bytes: usize) -> Option<(usize, usize)> {
    let msg_bits = msg_size_bytes.checked_mul(8)?;

    let (bit_start, bit_end) = match sig.byte_order() {
        ByteOrder::LittleEndian => le_start_end_bit(sig)?,
        ByteOrder::BigEndian => be_start_end_bit(sig)?,
    };

    if bit_start > msg_bits {
        warn!(
            signal = sig.name(),
            bit_start, msg_bits, "Signal start exceeds message size"
        );
        None
    } else if bit_end > msg_bits {
        warn!(
            signal = sig.name(),
            bit_end, msg_bits, "Signal end exceeds message size"
        );
        None
    } else {
        Some((bit_start, bit_end))
    }
}

fn be_start_end_bit(sig: &Signal) -> Option<(usize, usize)> {
    let x = sig.start_bit.checked_div(8)?;
    let x = x.checked_mul(8)?;

    let y = sig.start_bit.checked_rem(8)?;
    let y = 7u64.checked_sub(y)?;

    let start_bit = x.checked_add(y)?;
    let end_bit = start_bit.checked_add(sig.signal_size)?;

    Some((start_bit as usize, end_bit as usize))
}

fn le_start_end_bit(sig: &Signal) -> Option<(usize, usize)> {
    let start_bit = sig.start_bit;
    let end_bit = sig.start_bit.checked_add(sig.signal_size)?;
    Some((start_bit as usize, end_bit as usize))
}
