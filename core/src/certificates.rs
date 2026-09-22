use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use std::{
    error::Error,
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
use time::{Duration, OffsetDateTime};
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredAuthority {
    certificate: Vec<u8>,
    key: Vec<u8>,
}
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone)]
pub struct Authority {
    pub certificate: Vec<u8>,
    key: Arc<KeyPair>,
    issuer: Arc<Certificate>,
}
pub struct Leaf {
    pub certificate: Vec<u8>,
    pub key: Vec<u8>,
}
impl Authority {
    pub fn open(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let path = directory.join("authority.json");
        if !path.try_exists()? {
            let key = KeyPair::generate()?;
            let mut params = CertificateParams::default();
            params.distinguished_name = DistinguishedName::new();
            params
                .distinguished_name
                .push(DnType::CommonName, "GBF Flash Cache CA");
            params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
            params.not_before = OffsetDateTime::now_utc() - Duration::minutes(1);
            params.not_after = OffsetDateTime::now_utc() + Duration::days(3650);
            let certificate = params.self_signed(&key)?;
            let stored = StoredAuthority {
                certificate: certificate.der().to_vec(),
                key: key.serialize_der(),
            };
            let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
            temporary.write_all(&serde_json::to_vec(&stored)?)?;
            temporary.as_file().sync_all()?;
            // Another instance creating a CA must never replace the first instance's CA.
            match temporary.persist_noclobber(&path) {
                Ok(_) => {}
                Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(Box::new(e.error)),
            }
        }
        Self::from_bytes(&fs::read(path)?)
    }
    pub fn stored_certificate(directory: &Path) -> Result<Vec<u8>> {
        let bytes = fs::read(directory.join("authority.json"))?;
        if bytes.len() > 1024 * 1024 {
            return Err("CA store exceeds size limit".into());
        }
        let stored: StoredAuthority = serde_json::from_slice(&bytes)?;
        Ok(stored.certificate)
    }
    pub fn regenerate(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let staging = tempfile::tempdir_in(directory)?;
        let ca = Self::open(staging.path())?;
        let mut staged = tempfile::NamedTempFile::new_in(directory)?;
        staged.write_all(&fs::read(staging.path().join("authority.json"))?)?;
        staged.as_file().sync_all()?;
        staged.persist(directory.join("authority.json"))?;
        Ok(ca)
    }
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() > 1024 * 1024 {
            return Err("CA store exceeds size limit".into());
        }
        let StoredAuthority {
            certificate,
            key: private,
        } = serde_json::from_slice(data)?;
        let key = KeyPair::try_from(private.as_slice())?;
        let params = CertificateParams::from_ca_cert_der(&certificate.clone().into())?;
        let now = OffsetDateTime::now_utc();
        if now < params.not_before || now > params.not_after || !matches!(params.is_ca, IsCa::Ca(_))
        {
            return Err("CA is invalid or expired".into());
        }
        let (_, parsed) = x509_parser::parse_x509_certificate(&certificate)
            .map_err(|_| "invalid CA certificate")?;
        if parsed.public_key().raw != key.public_key_der() {
            return Err("CA private key does not match certificate".into());
        }
        parsed
            .verify_signature(None)
            .map_err(|_| "invalid CA signature")?;
        let issuer = params.self_signed(&key)?;
        Ok(Self {
            certificate,
            key: Arc::new(key),
            issuer: Arc::new(issuer),
        })
    }
    pub fn leaf(&self, host: &str) -> Result<Leaf> {
        let key = KeyPair::generate()?;
        let names = if host == "127.0.0.1" {
            vec![host.into(), "localhost".into()]
        } else {
            vec![host.into()]
        };
        let mut params = CertificateParams::new(names)?;
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, host);
        params.not_before = OffsetDateTime::now_utc() - Duration::minutes(1);
        params.not_after =
            (OffsetDateTime::now_utc() + Duration::days(365)).min(self.issuer.params().not_after);
        if params.not_after < OffsetDateTime::now_utc() {
            return Err("CA expired".into());
        }
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        Ok(Leaf {
            certificate: params
                .signed_by(&key, &self.issuer, &self.key)?
                .der()
                .to_vec(),
            key: key.serialize_der(),
        })
    }
}
