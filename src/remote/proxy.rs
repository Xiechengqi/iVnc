//! A restricted SOCKS5 bridge preserves Chrome's Miao proxy without exposing
//! host loopback/private services through the proxy path.
//!
//! Names are resolved here and Miao receives the checked numeric address. Passing
//! the name on would let a second lookup by Miao return a different, internal
//! address (DNS rebinding) and reach B's own services through a direct route.
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UnixStream},
};

pub fn public_destination(ip: Ipv4Addr, denied: &[IpAddr]) -> bool {
    let [a, b, _, _] = ip.octets();
    !ip.is_private()
        && !ip.is_loopback()
        && !ip.is_link_local()
        && !ip.is_broadcast()
        && !ip.is_documentation()
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && a != 0
        && a < 224
        && !(a == 100 && (64..=127).contains(&b))
        && !(a == 198 && (18..=19).contains(&b))
        && !denied.contains(&IpAddr::V4(ip))
}

pub fn public_ip(ip: IpAddr, denied: &[IpAddr]) -> bool {
    match ip {
        IpAddr::V4(ip) => public_destination(ip, denied),
        IpAddr::V6(ip) => public_v6(ip, denied),
    }
}

fn public_v6(ip: Ipv6Addr, denied: &[IpAddr]) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_destination(v4, denied);
    }
    let s = ip.segments();
    // NAT64 (64:ff9b::/96) embeds an IPv4 destination.
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return public_destination(v4, denied);
    }
    !ip.is_loopback()
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && (s[0] & 0xfe00) != 0xfc00 // unique local
        && (s[0] & 0xffc0) != 0xfe80 // link local
        && (s[0] & 0xffc0) != 0xfec0 // site local
        && !(s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        && s[0] != 0 // reserved, includes IPv4-compatible addresses
        && !denied.contains(&IpAddr::V6(ip))
}

enum Destination {
    Ip(IpAddr),
    Name(String),
}

/// Resolves and validates a destination. Every resolved address must be public,
/// and a failed lookup is refused rather than left to another resolver.
async fn resolve(destination: Destination, port: u16, denied: &[IpAddr]) -> Option<IpAddr> {
    let name = match destination {
        Destination::Ip(ip) => return public_ip(ip, denied).then_some(ip),
        Destination::Name(name) => name.trim_end_matches('.').to_ascii_lowercase(),
    };
    if name.is_empty()
        || name == "localhost"
        || name.ends_with(".localhost")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
    {
        return None;
    }
    let addresses: Vec<IpAddr> = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((name, port)),
    )
    .await
    .ok()?
    .ok()?
    .map(|a| a.ip())
    .collect();
    if addresses.is_empty() || !addresses.iter().all(|ip| public_ip(*ip, denied)) {
        return None;
    }
    addresses
        .iter()
        .find(|ip| ip.is_ipv4())
        .or_else(|| addresses.first())
        .copied()
}

pub async fn relay(mut client: UnixStream, upstream: SocketAddr, denied: Vec<IpAddr>) {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        if client.read_u8().await? != 5 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let count = client.read_u8().await? as usize;
        let mut methods = vec![0; count];
        client.read_exact(&mut methods).await?;
        if !methods.contains(&0) {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        client.write_all(&[5, 0]).await?;
        let mut head = [0; 4];
        client.read_exact(&mut head).await?;
        if head[..3] != [5, 1, 0] {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        let destination = match head[3] {
            1 => {
                let mut ip = [0; 4];
                client.read_exact(&mut ip).await?;
                Destination::Ip(IpAddr::from(ip))
            }
            3 => {
                let len = client.read_u8().await? as usize;
                let mut name = vec![0; len];
                client.read_exact(&mut name).await?;
                let name = String::from_utf8(name).map_err(|_| std::io::ErrorKind::InvalidData)?;
                match name.trim_matches(['[', ']']).parse::<IpAddr>() {
                    Ok(ip) => Destination::Ip(ip),
                    Err(_) => Destination::Name(name),
                }
            }
            4 => {
                let mut ip = [0; 16];
                client.read_exact(&mut ip).await?;
                Destination::Ip(IpAddr::from(ip))
            }
            _ => return Err(std::io::ErrorKind::PermissionDenied.into()),
        };
        let port = client.read_u16().await?;
        if port == 0 {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        let ip = resolve(destination, port, &denied)
            .await
            .ok_or(std::io::ErrorKind::PermissionDenied)?;
        let mut remote = TcpStream::connect(upstream).await?;
        remote.write_all(&[5, 1, 0]).await?;
        let mut reply = [0; 2];
        remote.read_exact(&mut reply).await?;
        if reply != [5, 0] {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        remote.write_all(&request(ip, port)).await?;
        Ok::<_, std::io::Error>(remote)
    })
    .await;
    match result {
        Ok(Ok(mut remote)) => {
            let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
        }
        _ => {
            let _ = client.write_all(&[5, 2, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        }
    }
}

fn request(ip: IpAddr, port: u16) -> Vec<u8> {
    let mut request = vec![5, 1, 0];
    match ip {
        IpAddr::V4(ip) => {
            request.push(1);
            request.extend(ip.octets());
        }
        IpAddr::V6(ip) => {
            request.push(4);
            request.extend(ip.octets());
        }
    }
    request.extend(port.to_be_bytes());
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_cannot_reach_internal_or_owner_addresses() {
        let denied = ["8.8.4.4".parse().unwrap(), "2001:4860::1".parse().unwrap()];
        for value in [
            "127.0.0.1",
            "10.0.2.2",
            "192.168.1.2",
            "172.16.1.2",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "224.0.0.1",
            "0.1.2.3",
            "8.8.4.4",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.2",
            "64:ff9b::a00:1",
            "2001:db8::1",
            "2001:4860::1",
            "::",
        ] {
            assert!(!public_ip(value.parse().unwrap(), &denied), "{value}");
        }
        for value in ["8.8.8.8", "2606:4700::1111", "64:ff9b::808:808"] {
            assert!(public_ip(value.parse().unwrap(), &denied), "{value}");
        }
    }

    #[tokio::test]
    async fn names_resolve_here_and_unresolvable_names_are_refused() {
        let name = |n: &str| Destination::Name(n.into());
        assert_eq!(resolve(name("localhost"), 80, &[]).await, None);
        assert_eq!(resolve(name("a.localhost"), 80, &[]).await, None);
        assert_eq!(resolve(name("bad name"), 80, &[]).await, None);
        // Never left to a second resolver, which could answer differently.
        assert_eq!(resolve(name("unresolvable.invalid"), 80, &[]).await, None);
        let public: IpAddr = "8.8.8.8".parse().unwrap();
        assert_eq!(
            resolve(Destination::Ip(public), 80, &[]).await,
            Some(public)
        );
        assert_eq!(resolve(Destination::Ip(public), 80, &[public]).await, None);
        assert_eq!(request(public, 443), [5, 1, 0, 1, 8, 8, 8, 8, 1, 187]);
        assert_eq!(request("2606:4700::1111".parse().unwrap(), 80)[3], 4);
    }

    #[tokio::test]
    async fn relay_sends_the_checked_address_upstream() {
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = upstream.local_addr().unwrap();
        let proxy = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            socket.write_all(&[5, 0]).await.unwrap();
            let mut request = vec![0; 10];
            socket.read_exact(&mut request).await.unwrap();
            socket
                .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
            request
        });
        let connect = |target: Vec<u8>| async move {
            let (mut app, bridge) = UnixStream::pair().unwrap();
            tokio::spawn(relay(bridge, address, Vec::new()));
            app.write_all(&[5, 1, 0]).await.unwrap();
            let mut choice = [0; 2];
            app.read_exact(&mut choice).await.unwrap();
            app.write_all(&target).await.unwrap();
            let mut reply = [0; 10];
            app.read_exact(&mut reply).await.unwrap();
            reply[1]
        };
        // A name that does not resolve here is refused before reaching Miao.
        let name = b"rebind.invalid";
        let mut by_name = vec![5, 1, 0, 3, name.len() as u8];
        by_name.extend(name);
        by_name.extend(443u16.to_be_bytes());
        assert_eq!(connect(by_name).await, 2);
        // An internal literal is refused; a public one is forwarded numerically.
        assert_eq!(connect(vec![5, 1, 0, 1, 127, 0, 0, 1, 0, 80]).await, 2);
        assert_eq!(connect(vec![5, 1, 0, 1, 8, 8, 8, 8, 1, 187]).await, 0);
        assert_eq!(proxy.await.unwrap(), [5, 1, 0, 1, 8, 8, 8, 8, 1, 187]);
    }
}
