use crate::net::TcpStream;

pub use async_native_tls::TlsAcceptor;
pub use async_native_tls::TlsConnector;
pub use async_native_tls::TlsStream;

// server both cliennt and server and same but use same pattern as rustls
pub type DefaultServerTlsStream = TlsStream<TcpStream>;
pub type DefaultClientTlsStream = TlsStream<TcpStream>;

pub use connector::*;
pub use stream::*;

mod connector {

    use std::io::Error as IoError;
    use std::io::ErrorKind;

    use std::os::unix::io::AsRawFd;
    use std::os::unix::io::RawFd;

    use async_native_tls::Error as NativeTlsError;
    use async_trait::async_trait;
    use log::debug;

    use crate::net::DefaultTcpDomainConnector;
    use crate::net::TcpDomainConnector;

    use super::AllTcpStream;
    use super::DefaultClientTlsStream;
    use super::TcpStream;
    use super::TlsConnector;

    pub enum TlsError {
        Io(IoError),
        Native(NativeTlsError),
    }

    impl From<IoError> for TlsError {
        fn from(error: IoError) -> Self {
            Self::Io(error)
        }
    }

    impl From<NativeTlsError> for TlsError {
        fn from(error: NativeTlsError) -> TlsError {
            Self::Native(error)
        }
    }

    /// connect as anonymous client
    pub struct TlsAnonymousConnector(TlsConnector);

    impl From<TlsConnector> for TlsAnonymousConnector {
        fn from(connector: TlsConnector) -> Self {
            Self(connector)
        }
    }

    #[async_trait]
    impl TcpDomainConnector for TlsAnonymousConnector {
        type WrapperStream = DefaultClientTlsStream;

        async fn connect(&self, domain: &str) -> Result<(Self::WrapperStream, RawFd), IoError> {
            let tcp_stream = TcpStream::connect(domain).await?;
            let fd = tcp_stream.as_raw_fd();
            let connector = self.0.connect(domain, tcp_stream).await.map_err(|e| {
                IoError::new(
                    ErrorKind::ConnectionRefused,
                    format!("failed to connect: {}", e),
                )
            })?;
            Ok((connector, fd))
        }
    }

    pub struct TlsDomainConnector {
        domain: String,
        connector: TlsConnector,
    }

    impl TlsDomainConnector {
        pub fn new(connector: TlsConnector, domain: String) -> Self {
            Self { domain, connector }
        }
    }

    #[async_trait]
    impl TcpDomainConnector for TlsDomainConnector {
        type WrapperStream = DefaultClientTlsStream;

        async fn connect(&self, addr: &str) -> Result<(Self::WrapperStream, RawFd), IoError> {
            debug!("connect to tls addr: {}", addr);
            let tcp_stream = TcpStream::connect(addr).await?;
            let fd = tcp_stream.as_raw_fd();

            debug!("connect to tls domain: {}", self.domain);
            let connector = self
                .connector
                .connect(&self.domain, tcp_stream)
                .await
                .map_err(|e| {
                    IoError::new(
                        ErrorKind::ConnectionRefused,
                        format!("failed to connect: {}", e),
                    )
                })?;
            Ok((connector, fd))
        }
    }

    pub enum AllDomainConnector {
        Tcp(DefaultTcpDomainConnector),
        TlsDomain(TlsDomainConnector),
        TlsAnonymous(TlsAnonymousConnector),
    }

    impl Default for AllDomainConnector {
        fn default() -> Self {
            Self::default_tcp()
        }
    }

    impl AllDomainConnector {
        pub fn default_tcp() -> Self {
            Self::Tcp(DefaultTcpDomainConnector::new())
        }

        pub fn new_tls_domain(connector: TlsDomainConnector) -> Self {
            Self::TlsDomain(connector)
        }

        pub fn new_tls_anonymous(connector: TlsAnonymousConnector) -> Self {
            Self::TlsAnonymous(connector)
        }
    }

    #[async_trait]
    impl TcpDomainConnector for AllDomainConnector {
        type WrapperStream = AllTcpStream;

        async fn connect(&self, domain: &str) -> Result<(Self::WrapperStream, RawFd), IoError> {
            match self {
                Self::Tcp(connector) => {
                    let (stream, fd) = connector.connect(domain).await?;
                    Ok((AllTcpStream::tcp(stream), fd))
                }

                Self::TlsDomain(connector) => {
                    let (stream, fd) = connector.connect(domain).await?;
                    Ok((AllTcpStream::tls(stream), fd))
                }
                Self::TlsAnonymous(connector) => {
                    let (stream, fd) = connector.connect(domain).await?;
                    Ok((AllTcpStream::tls(stream), fd))
                }
            }
        }
    }
}

pub use cert::*;

mod cert {
    use std::fs::File;
    use std::io::BufRead;
    use std::io::BufReader;
    use std::io::Error as IoError;
    use std::io::ErrorKind;
    use std::path::Path;

    use native_tls::Certificate as NativeCertificate;
    use native_tls::Identity;
    use openssl::pkcs12::Pkcs12;
    use openssl::pkey::Private;

    pub type Certificate = openssl::x509::X509;
    pub type PrivateKey = openssl::pkey::PKey<Private>;

    pub trait CertBuilder: Sized {
        fn new(bytes: Vec<u8>) -> Self;

        fn from_reader(reader: &mut dyn BufRead) -> Result<Self, IoError> {
            let mut bytes = vec![];
            reader.read_to_end(&mut bytes)?;
            Ok(Self::new(bytes))
        }

        fn from_path(path: impl AsRef<Path>) -> Result<Self, IoError> {
            let mut reader = BufReader::new(File::open(path)?);
            Self::from_reader(&mut reader)
        }
    }

    pub struct X509PemBuilder(Vec<u8>);

    impl CertBuilder for X509PemBuilder {
        fn new(bytes: Vec<u8>) -> Self {
            Self(bytes)
        }
    }

    impl X509PemBuilder {
        pub fn build(self) -> Result<Certificate, IoError> {
            let cert = Certificate::from_pem(&self.0).map_err(|err| {
                IoError::new(ErrorKind::InvalidInput, format!("invalid cert: {}", err))
            })?;
            Ok(cert)
        }

        pub fn build_native(self) -> Result<NativeCertificate, IoError> {
            NativeCertificate::from_pem(&self.0).map_err(|err| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid pem file: {}", err),
                )
            })
        }
    }

    pub struct PrivateKeyBuilder(Vec<u8>);

    impl CertBuilder for PrivateKeyBuilder {
        fn new(bytes: Vec<u8>) -> Self {
            Self(bytes)
        }
    }

    impl PrivateKeyBuilder {
        pub fn build(self) -> Result<PrivateKey, IoError> {
            let key = PrivateKey::private_key_from_pem(&self.0).map_err(|err| {
                IoError::new(ErrorKind::InvalidInput, format!("invalid key: {}", err))
            })?;
            Ok(key)
        }
    }

    const PASSWORD: &str = "test";

    pub struct IdentityBuilder(Vec<u8>);

    impl CertBuilder for IdentityBuilder {
        fn new(bytes: Vec<u8>) -> Self {
            Self(bytes)
        }
    }

    impl IdentityBuilder {
        /// load pk12 from x509 certs
        pub fn from_x509(x509: X509PemBuilder, key: PrivateKeyBuilder) -> Result<Self, IoError> {
            let server_key = key.build()?;
            let server_crt = x509.build()?;
            let p12 = Pkcs12::builder()
                .build(PASSWORD, "", &server_key, &server_crt)
                .map_err(|e| {
                    IoError::new(
                        ErrorKind::InvalidData,
                        format!("Failed to create Pkcs12: {}", e),
                    )
                })?;

            let der = p12.to_der()?;
            Ok(Self(der))
        }

        pub fn build(self) -> Result<Identity, IoError> {
            Identity::from_pkcs12(&self.0, PASSWORD).map_err(|e| {
                IoError::new(ErrorKind::InvalidData, format!("Failed to load der: {}", e))
            })
        }
    }
}

pub use builder::*;

mod builder {

    use std::io::Error as IoError;
    use std::io::ErrorKind;

    use native_tls::Identity;
    use native_tls::TlsAcceptor as NativeTlsAcceptor;

    use super::IdentityBuilder;
    use super::TlsAcceptor;
    use super::TlsConnector;
    use super::X509PemBuilder;

    pub struct ConnectorBuilder(TlsConnector);

    impl ConnectorBuilder {
        pub fn identity(builder: IdentityBuilder) -> Result<Self, IoError> {
            let identity = builder.build()?;
            let connector = TlsConnector::new().identity(identity);
            Ok(Self(connector))
        }

        pub fn anonymous() -> Self {
            let connector = TlsConnector::new()
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
            Self(connector)
        }

        pub fn no_cert_verification(self) -> Self {
            let connector = self.0.danger_accept_invalid_certs(true);
            Self(connector)
        }

        pub fn danger_accept_invalid_hostnames(self) -> Self {
            let connector = self.0.danger_accept_invalid_hostnames(true);
            Self(connector)
        }

        pub fn use_sni(self, use_sni: bool) -> Self {
            let connector = self.0.use_sni(use_sni);
            Self(connector)
        }

        pub fn add_root_certificate(self, builder: X509PemBuilder) -> Result<Self, IoError> {
            let certificate = builder.build_native()?;
            let connector = self.0.add_root_certificate(certificate);
            Ok(Self(connector))
        }

        pub fn build(self) -> TlsConnector {
            self.0
        }
    }

    pub struct AcceptorBuilder(Identity);

    impl AcceptorBuilder {
        pub fn identity(builder: IdentityBuilder) -> Result<Self, IoError> {
            let identity = builder.build()?;
            Ok(Self(identity))
        }

        pub fn build(self) -> Result<TlsAcceptor, IoError> {
            let acceptor = NativeTlsAcceptor::new(self.0)
                .map_err(|_| IoError::new(ErrorKind::InvalidInput, "invalid cert"))?;
            Ok(acceptor.into())
        }
    }
}

pub use stream::AllTcpStream;

mod stream {

    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use super::DefaultClientTlsStream;
    use super::TcpStream;
    use futures_lite::{AsyncRead, AsyncWrite};
    use pin_project::pin_project;

    #[pin_project(project = EnumProj)]
    pub enum AllTcpStream {
        Tcp(#[pin] TcpStream),
        Tls(#[pin] DefaultClientTlsStream),
    }

    impl AllTcpStream {
        pub fn tcp(stream: TcpStream) -> Self {
            Self::Tcp(stream)
        }

        pub fn tls(stream: DefaultClientTlsStream) -> Self {
            Self::Tls(stream)
        }
    }

    impl AsyncRead for AllTcpStream {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            match self.project() {
                EnumProj::Tcp(stream) => stream.poll_read(cx, buf),
                EnumProj::Tls(stream) => stream.poll_read(cx, buf),
            }
        }
    }

    impl AsyncWrite for AllTcpStream {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context,
            buf: &[u8],
        ) -> Poll<Result<usize, io::Error>> {
            match self.project() {
                EnumProj::Tcp(stream) => stream.poll_write(cx, buf),
                EnumProj::Tls(stream) => stream.poll_write(cx, buf),
            }
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
            match self.project() {
                EnumProj::Tcp(stream) => stream.poll_flush(cx),
                EnumProj::Tls(stream) => stream.poll_flush(cx),
            }
        }

        fn poll_close(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
            match self.project() {
                EnumProj::Tcp(stream) => stream.poll_close(cx),
                EnumProj::Tls(stream) => stream.poll_close(cx),
            }
        }
    }
}

#[cfg(test)]
mod test {

    use std::io::Error as IoError;
    use std::net::SocketAddr;
    use std::time;

    use async_native_tls::TlsAcceptor;
    use async_native_tls::TlsConnector;
    use bytes::buf::ext::BufExt;
    use bytes::BufMut;
    use bytes::Bytes;
    use bytes::BytesMut;
    use futures_lite::future::zip;
    use futures_lite::stream::StreamExt;
    use futures_util::sink::SinkExt;
    use log::debug;
    use tokio_util::codec::BytesCodec;
    use tokio_util::codec::Framed;
    use tokio_util::compat::FuturesAsyncReadCompatExt;

    use fluvio_future::net::TcpListener;
    use fluvio_future::net::TcpStream;
    use fluvio_future::test_async;
    use fluvio_future::timer::sleep;

    use super::{
        AcceptorBuilder, AllTcpStream, CertBuilder, ConnectorBuilder, IdentityBuilder,
        PrivateKeyBuilder, X509PemBuilder,
    };

    const CA_PATH: &str = "certs/certs/ca.crt";
    const SERVER_IDENTITY: &str = "certs/certs/server.pfx";
    const CLIENT_IDENTITY: &str = "certs/certs/client.pfx";
    const X509_SERVER: &str = "certs/certs/server.crt";
    const X509_SERVER_KEY: &str = "certs/certs/server.key";
    const X509_CLIENT: &str = "certs/certs/client.crt";
    const X509_CLIENT_KEY: &str = "certs/certs/client.key";

    fn to_bytes(bytes: Vec<u8>) -> Bytes {
        let mut buf = BytesMut::with_capacity(bytes.len());
        buf.put_slice(&bytes);
        buf.freeze()
    }

    #[test_async]
    async fn test_native_tls_pk12() -> Result<(), IoError> {
        const PK12_PORT: u16 = 9900;

        let acceptor = AcceptorBuilder::identity(
            IdentityBuilder::from_path(SERVER_IDENTITY).expect("identity"),
        )
        .expect("identity:")
        .build()
        .expect("acceptor");

        let connector = ConnectorBuilder::identity(IdentityBuilder::from_path(CLIENT_IDENTITY)?)
            .expect("connector")
            .danger_accept_invalid_hostnames()
            .no_cert_verification()
            .build();

        test_tls(PK12_PORT, acceptor, connector)
            .await
            .expect("no client cert test failed");

        let acceptor = AcceptorBuilder::identity(
            IdentityBuilder::from_path(SERVER_IDENTITY).expect("identity"),
        )
        .expect("identity:")
        .build()
        .expect("acceptor");

        let connector = ConnectorBuilder::identity(IdentityBuilder::from_path(CLIENT_IDENTITY)?)
            .expect("connector")
            .no_cert_verification()
            .build();

        test_tls(PK12_PORT, acceptor, connector)
            .await
            .expect("client cert test fail");

        Ok(())
    }

    #[test_async]
    async fn test_native_tls_x509() -> Result<(), IoError> {
        const X500_PORT: u16 = 9910;

        let acceptor = AcceptorBuilder::identity(
            IdentityBuilder::from_x509(
                X509PemBuilder::from_path(X509_SERVER).expect("read"),
                PrivateKeyBuilder::from_path(X509_SERVER_KEY).expect("file"),
            )
            .expect("identity"),
        )
        .expect("identity:")
        .build()
        .expect("acceptor");

        let connector = ConnectorBuilder::identity(
            IdentityBuilder::from_x509(
                X509PemBuilder::from_path(X509_CLIENT).expect("read"),
                PrivateKeyBuilder::from_path(X509_CLIENT_KEY).expect("read"),
            )
            .expect("509"),
        )
        .expect("connector")
        .danger_accept_invalid_hostnames()
        .no_cert_verification()
        .build();

        test_tls(X500_PORT, acceptor, connector)
            .await
            .expect("no client cert test failed");

        let acceptor = AcceptorBuilder::identity(
            IdentityBuilder::from_x509(
                X509PemBuilder::from_path(X509_SERVER).expect("read"),
                PrivateKeyBuilder::from_path(X509_SERVER_KEY).expect("file"),
            )
            .expect("identity"),
        )
        .expect("identity:")
        .build()
        .expect("acceptor");

        let connector = ConnectorBuilder::identity(
            IdentityBuilder::from_x509(
                X509PemBuilder::from_path(X509_CLIENT).expect("read"),
                PrivateKeyBuilder::from_path(X509_CLIENT_KEY).expect("read"),
            )
            .expect("509"),
        )
        .expect("connector")
        .add_root_certificate(X509PemBuilder::from_path(CA_PATH).expect("cert"))
        .expect("root")
        .no_cert_verification() // for mac
        .build();

        test_tls(X500_PORT, acceptor, connector)
            .await
            .expect("no client cert test failed");

        Ok(())
    }

    async fn test_tls(
        port: u16,
        acceptor: TlsAcceptor,
        connector: TlsConnector,
    ) -> Result<(), IoError> {
        const TEST_ITERATION: u16 = 2;

        let addr = format!("127.0.0.1:{}", port)
            .parse::<SocketAddr>()
            .expect("parse");

        let server_ft = async {
            debug!("server: binding");
            let listener = TcpListener::bind(&addr).await.expect("listener failed");
            debug!("server: successfully binding. waiting for incoming");

            let mut incoming = listener.incoming();
            let incoming_stream = incoming.next().await.expect("incoming");

            debug!("server: got connection from client");
            let tcp_stream = incoming_stream.expect("no stream");

            let tls_stream = acceptor.accept(tcp_stream).await.unwrap();
            let mut framed = Framed::new(tls_stream.compat(), BytesCodec::new());

            for _ in 0..TEST_ITERATION {
                debug!("server: sending values to client");
                let data = vec![0x05, 0x0a, 0x63];
                framed.send(to_bytes(data)).await.expect("send failed");
                sleep(time::Duration::from_micros(1)).await;
                debug!("server: sending 2nd value to client");
                let data2 = vec![0x20, 0x11];
                framed.send(to_bytes(data2)).await.expect("2nd send failed");
            }

            // sleep 1 seconds so we don't lost connection
            sleep(time::Duration::from_secs(1)).await;

            Ok(()) as Result<(), IoError>
        };

        let client_ft = async {
            debug!("client: sleep to give server chance to come up");
            sleep(time::Duration::from_millis(100)).await;
            debug!("client: trying to connect");
            let tcp_stream = TcpStream::connect(&addr).await.expect("connection fail");
            let tls_stream = connector
                .connect("localhost", tcp_stream)
                .await
                .expect("tls failed");
            let all_stream = AllTcpStream::Tls(tls_stream);
            let mut framed = Framed::new(all_stream.compat(), BytesCodec::new());
            debug!("client: got connection. waiting");

            for i in 0..TEST_ITERATION {
                let value = framed.next().await.expect("frame");
                debug!("{} client :received first value from server", i);
                let bytes = value.expect("invalid value");
                let values = bytes.take(3).into_inner();
                assert_eq!(values[0], 0x05);
                assert_eq!(values[1], 0x0a);
                assert_eq!(values[2], 0x63);
                assert_eq!(values.len(), 3);

                let value2 = framed.next().await.expect("frame");
                debug!("client: received 2nd value from server");
                let bytes = value2.expect("packet decoding works");
                let values = bytes.take(2).into_inner();
                assert_eq!(values.len(), 2);
            }

            Ok(()) as Result<(), IoError>
        };

        let _ = zip(client_ft, server_ft).await;

        Ok(())
    }
}
