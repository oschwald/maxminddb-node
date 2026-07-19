use crate::errors::invalid_arg;
use napi::{sys, Env, JsString, JsValue, Result};
use std::{
    net::{IpAddr, Ipv4Addr},
    str::FromStr,
};

pub(crate) fn parse_ip(ip: &str) -> Result<IpAddr> {
    if let Some(ip) = parse_ipv4(ip.as_bytes()) {
        return Ok(IpAddr::V4(ip));
    }
    IpAddr::from_str(ip).map_err(|_| invalid_arg(format!("Invalid IP address: {ip}")))
}

pub(crate) fn parse_js_ip(env: &Env, ip: JsString<'_>) -> Result<IpAddr> {
    // The longest valid textual IP address is shorter than this buffer. Read
    // normal inputs directly from V8 without allocating a Rust String. Fall
    // back to napi-rs' owned conversion only for overlong invalid inputs so
    // the existing error continues to include the complete value.
    let mut bytes = [0_u8; 64];
    let mut written = 0;
    let status = unsafe {
        sys::napi_get_value_string_utf8(
            env.raw(),
            ip.raw(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            &mut written,
        )
    };
    if status != sys::Status::napi_ok {
        return Err(invalid_arg("IP address must be a string"));
    }

    if written < bytes.len() - 1 {
        // Node's UTF-8 conversion always produces valid UTF-8, replacing
        // unpaired UTF-16 surrogates when necessary.
        return parse_ip(unsafe { std::str::from_utf8_unchecked(&bytes[..written]) });
    }

    let utf8 = ip.into_utf8()?;
    parse_ip(utf8.as_str()?)
}

pub(crate) fn parse_network(cidr: &str) -> Result<ipnetwork::IpNetwork> {
    ipnetwork::IpNetwork::from_str(cidr)
        .map_err(|err| invalid_arg(format!("Invalid network CIDR '{cidr}': {err}")))
}

fn parse_ipv4(bytes: &[u8]) -> Option<Ipv4Addr> {
    let mut octets = [0_u8; 4];
    let mut octet_index = 0;
    let mut value: u16 = 0;
    let mut digits = 0;

    for &byte in bytes {
        if byte == b'.' {
            if digits == 0 || octet_index == 3 {
                return None;
            }
            octets[octet_index] = value as u8;
            octet_index += 1;
            value = 0;
            digits = 0;
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        if digits == 1 && value == 0 {
            return None;
        }
        digits += 1;
        if digits > 3 {
            return None;
        }
        value = value * 10 + u16::from(byte - b'0');
        if value > u16::from(u8::MAX) {
            return None;
        }
    }

    if octet_index != 3 || digits == 0 {
        return None;
    }
    octets[octet_index] = value as u8;
    Some(Ipv4Addr::from(octets))
}

pub(crate) fn prefix_len_for_lookup(ip: IpAddr, network: ipnetwork::IpNetwork) -> usize {
    if ip.is_ipv4() && network.is_ipv6() {
        (network.prefix() as usize).saturating_sub(96)
    } else {
        network.prefix() as usize
    }
}
