//! A restricted SOCKS5 bridge preserves Chrome's Miao proxy without exposing
//! host loopback/private services through the proxy path.
use std::{
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UnixStream},
};

pub fn public_destination(ip: Ipv4Addr, denied: &[Ipv4Addr]) -> bool {
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
        && !denied.contains(&ip)
}

pub async fn relay(mut client: UnixStream, upstream: SocketAddr, denied: Vec<Ipv4Addr>) {
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
        let host = match head[3] {
            1 => {
                let mut ip = [0; 4];
                client.read_exact(&mut ip).await?;
                Ipv4Addr::from(ip).to_string()
            }
            3 => {
                let len = client.read_u8().await? as usize;
                let mut name = vec![0; len];
                client.read_exact(&mut name).await?;
                String::from_utf8(name).map_err(|_| std::io::ErrorKind::InvalidData)?
            }
            _ => return Err(std::io::ErrorKind::PermissionDenied.into()),
        };
        let port = client.read_u16().await?;
        if port == 0 {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        let addresses: Vec<_> = tokio::net::lookup_host((host.as_str(), port))
            .await?
            .collect();
        // Resolve here and send a numeric IP upstream, preventing a second DNS
        // lookup by the proxy from changing a validated destination.
        let ip = addresses
            .iter()
            .filter_map(|a| match a.ip() {
                std::net::IpAddr::V4(ip) => Some(ip),
                _ => None,
            })
            .find(|ip| public_destination(*ip, &denied))
            .ok_or(std::io::ErrorKind::PermissionDenied)?;
        if addresses.iter().any(|a| match a.ip() {
            std::net::IpAddr::V4(ip) => !public_destination(ip, &denied),
            _ => false,
        }) {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        let mut remote = TcpStream::connect(upstream).await?;
        remote.write_all(&[5, 1, 0]).await?;
        let mut reply = [0; 2];
        remote.read_exact(&mut reply).await?;
        if reply != [5, 0] {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        let mut request = vec![5, 1, 0, 1];
        request.extend(ip.octets());
        request.extend(port.to_be_bytes());
        remote.write_all(&request).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_cannot_reach_internal_or_owner_addresses() {
        let denied = ["8.8.4.4".parse().unwrap()];
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
        ] {
            assert!(
                !public_destination(value.parse().unwrap(), &denied),
                "{value}"
            );
        }
        assert!(public_destination("8.8.8.8".parse().unwrap(), &denied));
    }
}
