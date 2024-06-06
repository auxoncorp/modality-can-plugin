use nom::{
    branch::alt,
    bytes::complete::{tag, take},
    character::complete::{alphanumeric1, digit1, space0},
    combinator::{map, map_opt, map_res, opt},
    multi::fold_many0,
    sequence::{delimited, preceded, separated_pair, terminated, tuple},
    IResult,
};
use socketcan::{
    frame::FdFlags, CanAnyFrame, CanDataFrame, CanFdFrame, CanRemoteFrame, EmbeddedFrame,
    ExtendedId, Id as CanId, StandardId, Timestamp,
};

pub const SOF: char = '(';

const CANID_DELIM: &str = "#";
const DATA_SEPERATOR: &str = ".";

pub type CanInterface<'a> = &'a str;

pub fn parse(s: &str) -> IResult<&str, (Timestamp, CanInterface<'_>, CanAnyFrame)> {
    tuple((
        timestamp,
        preceded(space0, interface),
        preceded(space0, can_any_frame),
    ))(s)
}

fn timestamp(s: &str) -> IResult<&str, Timestamp> {
    map(
        delimited(
            tag("("),
            separated_pair(
                map_res(digit1, |out: &str| out.parse::<i64>()),
                tag("."),
                map_res(digit1, |out: &str| out.parse::<i64>()),
            ),
            tag(")"),
        ),
        |(seconds, nanoseconds)| Timestamp {
            seconds,
            nanoseconds,
        },
    )(s)
}

fn interface(s: &str) -> IResult<&str, &str> {
    alphanumeric1(s)
}

fn can_any_frame(s: &str) -> IResult<&str, CanAnyFrame> {
    alt((
        map(can_remote_frame, CanAnyFrame::Remote),
        map(can_fd_frame, CanAnyFrame::Fd),
        map(can_data_frame, CanAnyFrame::Normal),
    ))(s)
}

fn can_remote_frame(s: &str) -> IResult<&str, CanRemoteFrame> {
    map_opt(
        tuple((
            can_id,
            preceded(
                alt((tag("R"), tag("r"))),
                opt(map_res(digit1, |out: &str| usize::from_str_radix(out, 16))),
            ),
        )),
        |(id, maybe_dlc)| CanRemoteFrame::new_remote(id, maybe_dlc.unwrap_or(0)),
    )(s)
}

fn can_data_frame(s: &str) -> IResult<&str, CanDataFrame> {
    map_opt(tuple((can_id, hex_data)), |(id, data)| {
        CanDataFrame::new(id, &data)
    })(s)
}

fn can_fd_frame(s: &str) -> IResult<&str, CanFdFrame> {
    map_opt(
        tuple((can_id, preceded(tag("#"), fd_flags), hex_data)),
        |(id, flags, data)| CanFdFrame::with_flags(id, &data, flags),
    )(s)
}

fn fd_flags(s: &str) -> IResult<&str, FdFlags> {
    map_opt(take(1_usize), |flags: &str| {
        u8::from_str_radix(flags, 16)
            .ok()
            .map(FdFlags::from_bits_truncate)
    })(s)
}

fn hex_data(s: &str) -> IResult<&str, Vec<u8>> {
    fold_many0(
        map_res(
            terminated(take(2_usize), opt(tag(DATA_SEPERATOR))),
            |out: &str| u8::from_str_radix(out, 16),
        ),
        Vec::new,
        |mut acc: Vec<_>, byte| {
            acc.push(byte);
            acc
        },
    )(s)
}

fn can_id(s: &str) -> IResult<&str, CanId> {
    alt((
        map_opt(terminated(take(8_usize), tag(CANID_DELIM)), |out: &str| {
            u32::from_str_radix(out, 16)
                .ok()
                .and_then(ExtendedId::new)
                .map(CanId::from)
        }),
        // CAN XL just gets converted to ExtendedId
        map_opt(terminated(take(5_usize), tag(CANID_DELIM)), |out: &str| {
            u32::from_str_radix(out, 16)
                .ok()
                .and_then(ExtendedId::new)
                .map(CanId::from)
        }),
        map_opt(terminated(take(3_usize), tag(CANID_DELIM)), |out: &str| {
            u16::from_str_radix(out, 16)
                .ok()
                .and_then(StandardId::new)
                .map(CanId::from)
        }),
    ))(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use socketcan::EmbeddedFrame;

    #[test]
    fn timestamp_parser() {
        assert_eq!(
            timestamp("(000000.000001)"),
            Ok((
                "",
                Timestamp {
                    seconds: 0,
                    nanoseconds: 1,
                }
            ))
        );
        assert_eq!(
            timestamp("(1717673394.196203)"),
            Ok((
                "",
                Timestamp {
                    seconds: 1717673394,
                    nanoseconds: 196203,
                }
            ))
        );
    }

    #[test]
    fn interface_parser() {
        assert_eq!(interface("vcan0"), Ok(("", "vcan0")));
    }

    #[test]
    fn can_id_parser() {
        assert_eq!(
            can_id("00A#"),
            Ok(("", StandardId::new(0xA).unwrap().into()))
        );
        assert_eq!(
            can_id("10A#"),
            Ok(("", StandardId::new(0x10A).unwrap().into()))
        );
        assert_eq!(
            can_id("00BCD#"),
            Ok(("", ExtendedId::new(0x00BCD).unwrap().into()))
        );
        assert_eq!(
            can_id("1BED56DA#"),
            Ok(("", ExtendedId::new(0x1BED56DA).unwrap().into()))
        );
    }

    #[test]
    fn can_remote_frame_parser() {
        let (rem, f) = can_remote_frame("1C9#R8").unwrap();
        assert!(rem.is_empty());
        assert_eq!(f.id(), StandardId::new(0x1C9).unwrap().into());
        assert!(f.is_remote_frame());
        assert_eq!(f.dlc(), 8);

        let (rem, f) = can_remote_frame("25D#R").unwrap();
        assert!(rem.is_empty());
        assert_eq!(f.id(), StandardId::new(0x25D).unwrap().into());
        assert!(f.is_remote_frame());
        assert_eq!(f.dlc(), 0);
    }

    #[test]
    fn can_fd_frame_parser() {
        let (rem, f) = can_fd_frame("6BD##4").unwrap();
        assert!(rem.is_empty());
        assert_eq!(f.id(), StandardId::new(0x6BD).unwrap().into());
        assert!(f.is_data_frame());
        assert_eq!(f.flags(), FdFlags::empty());
        assert_eq!(f.dlc(), 0);
        assert_eq!(f.data(), &[]);

        let (rem, f) = can_fd_frame("17F4200A##410").unwrap();
        assert!(rem.is_empty());
        assert_eq!(f.id(), ExtendedId::new(0x17F4200A).unwrap().into());
        assert!(f.is_data_frame());
        assert_eq!(f.flags(), FdFlags::empty());
        assert_eq!(f.dlc(), 1);
        assert_eq!(f.data(), &[0x10]);

        let (rem, f) = can_fd_frame("2CD##53A").unwrap();
        assert!(rem.is_empty());
        assert_eq!(f.id(), StandardId::new(0x2CD).unwrap().into());
        assert!(f.is_data_frame());
        assert_eq!(f.flags(), FdFlags::BRS);
        assert_eq!(f.dlc(), 1);
        assert_eq!(f.data(), &[0x3A]);

        let (rem, f) = can_fd_frame("7AE##7EE").unwrap();
        assert!(rem.is_empty());
        assert_eq!(f.id(), StandardId::new(0x7AE).unwrap().into());
        assert!(f.is_data_frame());
        assert_eq!(f.flags(), FdFlags::BRS | FdFlags::ESI);
        assert_eq!(f.dlc(), 1);
        assert_eq!(f.data(), &[0xEE]);
    }

    #[test]
    fn can_data_frame_parser() {
        let (rem, f) = can_data_frame("6BD#").unwrap();
        assert!(rem.is_empty());
        assert_eq!(f.id(), StandardId::new(0x6BD).unwrap().into());
        assert!(f.is_data_frame());
        assert_eq!(f.dlc(), 0);
        assert_eq!(f.data(), &[]);

        let (rem, f) = can_data_frame("27E#39.DB").unwrap();
        assert!(rem.is_empty());
        assert_eq!(f.id(), StandardId::new(0x27E).unwrap().into());
        assert!(f.is_data_frame());
        assert_eq!(f.dlc(), 2);
        assert_eq!(f.data(), &[0x39, 0xDB]);
    }

    #[test]
    fn parser() {
        let (rem, (ts, iface, frame)) = parse("(1717689368.527737) vcan0 18A#F47E").unwrap();
        assert!(rem.is_empty());
        assert_eq!(
            ts,
            Timestamp {
                seconds: 1717689368,
                nanoseconds: 527737,
            }
        );
        assert_eq!(iface, "vcan0");
        let f = if let CanAnyFrame::Normal(f) = frame {
            f
        } else {
            panic!();
        };
        assert_eq!(f.id(), StandardId::new(0x18A).unwrap().into());
        assert!(f.is_data_frame());
        assert_eq!(f.dlc(), 2);
        assert_eq!(f.data(), &[0xF4, 0x7E]);
    }
}
