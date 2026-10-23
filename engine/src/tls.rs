//! TLS for remote connections (rustls with the ring provider).

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::client::TlsMode;
use crate::error::{DepError, Result};

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The client configuration for a TLS mode: `require` encrypts without checking the
/// certificate; `verify-full` checks it against the public roots and the CA file, if any.
pub fn client_config(mode: TlsMode, ca_file: Option<&str>) -> Result<ClientConfig> {
    let builder = ClientConfig::builder_with_provider(provider()).with_safe_default_protocol_versions().map_err(|e| DepError::sql("08000", e.to_string()))?;
    let cfg = match mode {
        TlsMode::Require => builder.dangerous().with_custom_certificate_verifier(Arc::new(NoVerify(provider()))).with_no_client_auth(),
        _ => {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            if let Some(path) = ca_file.filter(|p| !p.is_empty()) {
                let certs = CertificateDer::pem_file_iter(path).map_err(|e| DepError::sql("58030", format!("could not read CA file {path}: {e}")))?;
                for c in certs {
                    let c = c.map_err(|e| DepError::sql("58030", format!("could not read CA file {path}: {e}")))?;
                    roots.add(c).map_err(|e| DepError::sql("58030", format!("invalid certificate in {path}: {e}")))?;
                }
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        }
    };
    Ok(cfg)
}

/// Accepts any server certificate: the `require` mode (encryption without authentication).
#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The server side: a TLS acceptor for the wire protocol, from PEM certificate and key files.
pub fn server_acceptor(cert: &std::path::Path, key: &std::path::Path) -> Result<pgwire::tokio::TlsAcceptor> {
    use rustls::pki_types::PrivateKeyDer;
    let read_err = |p: &std::path::Path, e: String| DepError::sql("58030", format!("could not read {}: {e}", p.display()));
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert).map_err(|e| read_err(cert, e.to_string()))?.collect::<std::result::Result<_, _>>().map_err(|e| read_err(cert, e.to_string()))?;
    let key_der = PrivateKeyDer::from_pem_file(key).map_err(|e| read_err(key, e.to_string()))?;
    let cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| DepError::sql("08000", e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(chain, key_der)
        .map_err(|e| DepError::sql("08000", format!("invalid TLS certificate or key: {e}")))?;
    Ok(pgwire::tokio::TlsAcceptor::from(Arc::new(cfg)))
}

/// Creates a self-signed certificate for `hosts` in `dir` (server.crt, server.key), unless one is there already.
/// Clients connect to it with sslmode=require, or with verify-full and the certificate as their CA file.
pub fn ensure_self_signed(dir: &std::path::Path, hosts: &[String]) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let cert = dir.join("server.crt");
    let key = dir.join("server.key");
    if cert.exists() && key.exists() {
        return Ok((cert, key));
    }
    std::fs::create_dir_all(dir).map_err(|e| DepError::sql("58030", e.to_string()))?;
    let mut names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    names.extend(hosts.iter().cloned());
    names.dedup();
    let ck = rcgen::generate_simple_self_signed(names).map_err(|e| DepError::sql("58030", e.to_string()))?;
    std::fs::write(&cert, ck.cert.pem()).map_err(|e| DepError::sql("58030", e.to_string()))?;
    std::fs::write(&key, ck.key_pair.serialize_pem()).map_err(|e| DepError::sql("58030", e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600));
    }
    Ok((cert, key))
}
