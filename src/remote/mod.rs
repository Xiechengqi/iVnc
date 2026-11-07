//! Application-scoped desktop sharing. Web and CLI applications are never exported.
pub mod gateway;
pub mod media;
pub mod proxy;
pub mod store;
pub mod worker;

pub const PROTOCOL: u32 = 1;

pub fn token() -> String {
    use rand_core::RngCore;
    let mut bytes = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
