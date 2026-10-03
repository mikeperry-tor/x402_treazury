//! Authenticated loopback SOCKS fixture; never resolves a destination hostname.
#![allow(dead_code)]
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Copy)]
pub enum Fault {
    None,
    NoAuth,
    BadAuth,
    Refuse,
    Stall,
}
#[derive(Clone, Debug)]
pub struct Record {
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: u16,
    pub address_type: u8,
    pub upstream_peer: Option<SocketAddr>,
}
pub struct Socks {
    pub address: SocketAddr,
    pub records: Arc<Mutex<Vec<Record>>>,
    stop: CancellationToken,
}
impl Drop for Socks {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl Socks {
    pub async fn start(routes: BTreeMap<String, SocketAddr>, fault: Fault) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let records = Arc::new(Mutex::new(Vec::new()));
        let stop = CancellationToken::new();
        let (logs, cancel) = (records.clone(), stop.clone());
        tokio::spawn(async move {
            loop {
                let socket = tokio::select! { _ = cancel.cancelled() => break, result = listener.accept() => match result { Ok((s,_)) => s, Err(_) => break } };
                let (records, routes, stop) = (logs.clone(), routes.clone(), cancel.clone());
                tokio::spawn(async move {
                    tokio::select! { _ = stop.cancelled() => {}, _ = serve(socket, routes, fault, records) => {} }
                });
            }
        });
        Self {
            address,
            records,
            stop,
        }
    }
}
async fn string(socket: &mut TcpStream) -> std::io::Result<String> {
    let size = socket.read_u8().await?;
    let mut bytes = vec![0; size as usize];
    socket.read_exact(&mut bytes).await?;
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("invalid string"))
}
async fn serve(
    mut socket: TcpStream,
    routes: BTreeMap<String, SocketAddr>,
    fault: Fault,
    records: Arc<Mutex<Vec<Record>>>,
) -> std::io::Result<()> {
    if matches!(fault, Fault::Stall) {
        std::future::pending::<()>().await;
    }
    if socket.read_u8().await? != 5 {
        return Err(std::io::Error::other("not socks5"));
    }
    let count = socket.read_u8().await?;
    let mut methods = vec![0; count as usize];
    socket.read_exact(&mut methods).await?;
    if methods != [2] {
        return Err(std::io::Error::other("authentication must be mandatory"));
    }
    socket
        .write_all(&[5, if matches!(fault, Fault::NoAuth) { 0 } else { 2 }])
        .await?;
    if matches!(fault, Fault::NoAuth) {
        return Ok(());
    }
    if socket.read_u8().await? != 1 {
        return Err(std::io::Error::other("not RFC1929"));
    }
    let user = string(&mut socket).await?;
    let password = string(&mut socket).await?;
    socket
        .write_all(&[1, u8::from(matches!(fault, Fault::BadAuth))])
        .await?;
    if matches!(fault, Fault::BadAuth) {
        return Ok(());
    }
    let mut head = [0; 4];
    socket.read_exact(&mut head).await?;
    if head[..3] != [5, 1, 0] {
        return Err(std::io::Error::other("not CONNECT"));
    }
    let host = match head[3] {
        3 => string(&mut socket).await?,
        1 => {
            let mut b = [0; 4];
            socket.read_exact(&mut b).await?;
            std::net::Ipv4Addr::from(b).to_string()
        }
        4 => {
            let mut b = [0; 16];
            socket.read_exact(&mut b).await?;
            std::net::Ipv6Addr::from(b).to_string()
        }
        _ => return Err(std::io::Error::other("invalid address type")),
    };
    let port = socket.read_u16().await?;
    let record_index = {
        let mut logs = records.lock().unwrap();
        let index = logs.len();
        logs.push(Record {
            user,
            password,
            host: host.clone(),
            port,
            address_type: head[3],
            upstream_peer: None,
        });
        index
    };
    if matches!(fault, Fault::Refuse) {
        socket.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    }
    let loopback = host
        .parse::<std::net::IpAddr>()
        .ok()
        .filter(|ip| ip.is_loopback() && routes.contains_key("loopback"))
        .map(|ip| SocketAddr::new(ip, port));
    let target = routes
        .get(&format!("{host}:{port}"))
        .or_else(|| routes.get(&host))
        .or(loopback.as_ref());
    let Some(target) = target else {
        socket.write_all(&[5, 4, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    };
    let mut remote = TcpStream::connect(target).await?;
    records.lock().unwrap()[record_index].upstream_peer = Some(remote.local_addr()?);
    socket.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await?;
    tokio::io::copy_bidirectional(&mut socket, &mut remote).await?;
    Ok(())
}
