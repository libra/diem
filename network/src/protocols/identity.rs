// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! Protocol used to identify key information about a remote
//!
//! Currently, the information shared as part of this protocol includes the peer identity and a
//! list of protocols supported by the peer.
use crate::protocols::wire::handshake::v1::HandshakeMsg;
use bytes::BytesMut;
use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use libra_types::PeerId;
use netcore::framing::{read_u16frame, write_u16frame};
use std::io;

/// The PeerId exchange protocol.
pub async fn exchange_peerid<T>(own_peer_id: &PeerId, socket: &mut T) -> io::Result<PeerId>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    // Send serialized PeerId to remote peer.
    let msg = lcs::to_bytes(own_peer_id).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to serialize identity msg: {}", e),
        )
    })?;
    write_u16frame(socket, &msg).await?;
    socket.flush().await?;

    // Read PeerId from remote peer.
    let mut response = BytesMut::new();
    read_u16frame(socket, &mut response).await?;
    let remote_peer_id = lcs::from_bytes(&response).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse PeerId msg: {}", e),
        )
    })?;
    Ok(remote_peer_id)
}

#[allow(unused)]
pub fn sync_peer_id_exchange<T>(own_peer_id: &PeerId, socket: &mut T) -> io::Result<PeerId>
where
    T: io::Read + io::Write,
{
    // Send serialized PeerId to remote peer.
    let msg = lcs::to_bytes(own_peer_id).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to serialize identity msg: {}", e),
        )
    })?;
    // write length:
    let len_bytes = u16::to_be_bytes(msg.len() as u16);
    socket.write_all(&len_bytes)?;
    socket.write_all(&msg)?;
    socket.flush()?;

    // Read PeerId from remote peer.
    let mut len_bytes = [0u8; 2];
    socket.read_exact(&mut len_bytes)?;
    let len = u16::from_be_bytes(len_bytes);
    let mut response = vec![0u8; len as usize];
    socket.read_exact(&mut response)?;
    let remote_peer_id = lcs::from_bytes(&response).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse PeerId msg: {}", e),
        )
    })?;
    Ok(remote_peer_id)
}

/// The Handshake exchange protocol.
pub async fn exchange_handshake<T>(
    own_handshake: &HandshakeMsg,
    socket: &mut T,
) -> io::Result<HandshakeMsg>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    // Send serialized handshake message to remote peer.
    let msg = lcs::to_bytes(own_handshake).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to serialize identity msg: {}", e),
        )
    })?;
    write_u16frame(socket, &msg).await?;
    socket.flush().await?;

    // Read handshake message from the Remote
    let mut response = BytesMut::new();
    read_u16frame(socket, &mut response).await?;
    let identity = lcs::from_bytes(&response).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse Handshake msg: {}", e),
        )
    })?;
    Ok(identity)
}

/// The Handshake exchange protocol.
#[allow(unused)]
pub fn sync_exchange_handshake<T>(
    own_handshake: &HandshakeMsg,
    socket: &mut T,
) -> io::Result<HandshakeMsg>
where
    T: io::Read + io::Write,
{
    // Send serialized handshake message to remote peer.
    let msg = lcs::to_bytes(own_handshake).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to serialize identity msg: {}", e),
        )
    })?;
    // write length:
    let len_bytes = u16::to_be_bytes(msg.len() as u16);
    socket.write_all(&len_bytes)?;
    socket.write_all(&msg)?;
    socket.flush()?;

    // Read handshake message from the Remote
    let mut len_bytes = [0u8; 2];
    socket.read_exact(&mut len_bytes)?;
    let len = u16::from_be_bytes(len_bytes);
    let mut response = vec![0u8; len as usize];
    socket.read_exact(&mut response)?;
    let identity = lcs::from_bytes(&response).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse Handshake msg: {}", e),
        )
    })?;
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use crate::{
        protocols::{
            identity::{
                exchange_handshake, exchange_peerid, sync_exchange_handshake, sync_peer_id_exchange,
            },
            wire::handshake::v1::{HandshakeMsg, MessagingProtocolVersion},
        },
        ProtocolId,
    };
    use futures::{executor::block_on, future::join};
    use libra_types::PeerId;
    use memsocket::MemorySocket;
    use netcore::{compat::IoCompat, transport::ConnectionOrigin};
    use std::net::TcpListener;
    use tokio::net::TcpStream;

    fn build_test_connection() -> (MemorySocket, MemorySocket) {
        MemorySocket::new_pair()
    }

    #[tokio::test]
    async fn simple_handshake() {
        let (mut outbound, mut inbound) = build_test_connection();

        // Create client and server handshake messages.
        let mut server_handshake = HandshakeMsg::new();
        server_handshake.add(
            MessagingProtocolVersion::V1,
            [
                ProtocolId::ConsensusDirectSend,
                ProtocolId::MempoolDirectSend,
            ]
            .iter()
            .into(),
        );
        let mut client_handshake = HandshakeMsg::new();
        client_handshake.add(
            MessagingProtocolVersion::V1,
            [ProtocolId::ConsensusRpc, ProtocolId::ConsensusDirectSend]
                .iter()
                .into(),
        );

        let server_handshake_clone = server_handshake.clone();
        let client_handshake_clone = client_handshake.clone();

        let server = async move {
            let handshake = exchange_handshake(&server_handshake, &mut inbound)
                .await
                .expect("Handshake fails");

            assert_eq!(
                lcs::to_bytes(&handshake).unwrap(),
                lcs::to_bytes(&client_handshake_clone).unwrap()
            );
        };

        let client = async move {
            let handshake = exchange_handshake(&client_handshake, &mut outbound)
                .await
                .expect("Handshake fails");

            assert_eq!(
                lcs::to_bytes(&handshake).unwrap(),
                lcs::to_bytes(&server_handshake_clone).unwrap()
            );
        };

        block_on(join(server, client));
    }

    #[tokio::test]
    async fn simple_peerid_exchange() {
        let (mut outbound, mut inbound) = build_test_connection();

        // Create client and server ids.
        let client_id = PeerId::random();
        let server_id = PeerId::random();

        let server = async {
            let id = exchange_peerid(&server_id, &mut inbound)
                .await
                .expect("Identity exchange fails");

            assert_eq!(id, client_id);
        };

        let client = async {
            let id = exchange_peerid(&client_id, &mut outbound)
                .await
                .expect("Identity exchange fails");

            assert_eq!(id, server_id);
        };

        join(server, client).await;
    }

    #[tokio::test(core_threads = 10)]
    async fn simple_id_handshake_sync_async_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let listen_addr = listener.local_addr().unwrap();

        let server_peer_id = PeerId::random();
        let client_peer_id = PeerId::random();

        // Create client and server handshake messages.
        let mut server_handshake = HandshakeMsg::new();
        server_handshake.add(
            MessagingProtocolVersion::V1,
            [
                ProtocolId::ConsensusDirectSend,
                ProtocolId::MempoolDirectSend,
            ]
            .iter()
            .into(),
        );
        let mut client_handshake = HandshakeMsg::new();
        client_handshake.add(
            MessagingProtocolVersion::V1,
            [ProtocolId::ConsensusRpc, ProtocolId::ConsensusDirectSend]
                .iter()
                .into(),
        );

        let server_handshake_clone = server_handshake.clone();
        let client_handshake_clone = client_handshake.clone();

        // Async identity exchange at server using tokio::net::TcpStream.
        let server = async move {
            let inbound = listener.accept().unwrap().0;
            let mut inbound = IoCompat::new(TcpStream::from_std(inbound).unwrap());
            let peer_id = exchange_peerid(&server_peer_id, &mut inbound)
                .await
                .expect("PeerId exchange fails");
            assert_eq!(peer_id, client_peer_id);
            let handshake = exchange_handshake(&server_handshake, &mut inbound)
                .await
                .expect("Handshake exchange fails");
            assert_eq!(handshake, client_handshake_clone);
        };
        tokio::spawn(server);

        // Sync Identity exchange at client using std::net::TcpStream.
        let mut outbound = std::net::TcpStream::connect(listen_addr).unwrap();
        let peer_id =
            sync_peer_id_exchange(&client_peer_id, &mut outbound).expect("PeerId exchange fails");
        assert_eq!(peer_id, server_peer_id);
        let handshake = sync_exchange_handshake(&client_handshake, &mut outbound)
            .expect("Handshake exchange fails");
        assert_eq!(handshake, server_handshake_clone);
    }
}
