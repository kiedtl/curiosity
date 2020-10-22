use url::{Url, ParseError};
use std::sync::Arc;
use tokio::net::{
    TcpStream
};
use tokio_rustls::TlsConnector;
use gemtext::*;
use std::error::Error;
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let urlstr = "gemini://gemini.circumlunar.space:1965/";

    let mut cfg = tokio_rustls::rustls::ClientConfig::new();
    cfg
        .dangerous()
        .set_certificate_verifier(Arc::new(NoCertificateVerification {}));


    crawl(urlstr, cfg)?;
    Ok(())
}

fn crawl(start: &str, cfg: tokio_rustls::rustls::ClientConfig)
    -> Result<(), Box<dyn Error>>
{
    let start = parse_url(None, start)?;

    // map of visited urls
    let mut visited: HashMap<String, bool> = HashMap::new();

    // queue to visit
    let mut queue: Vec<Url> = Vec::new();

    // start crawling with the first url
    let response = smol::run(get(start.clone(), cfg.clone()))?;
    let urls = extract(start, response);

    for url in &urls {
        queue.push(url.clone());
        visited.insert(url.to_string(), true);
    }

    const MAX: usize = 20;

    // main crawl
    while queue.len() > 0 && visited.len() < MAX {
        let url = queue.pop().unwrap();

        let response = smol::run(get(url.clone(), cfg.clone()))?;
        let urls = extract(url, response);

        for url in &urls {
            if visited.get(&url.to_string())
                .unwrap_or(&false) == &false {
                    queue.push(url.clone());
                    visited.insert(url.to_string(), true);

                    if visited.len() >= MAX {
                        break;
                    }
            }
        }
    }

    println!("data({}): {:#?}", visited.len(),
        visited.iter().map(|u| u.0).collect::<Vec<_>>());
    Ok(())
}


fn extract(base_url: Url, data: Vec<u8>) -> Vec<Url> {
    let data_s = data.iter().map(|b| *b as char)
        .collect::<String>();
    let parsed = gemtext::parse(&data_s);

    let mut found = Vec::new();

    for node in parsed {
        match node {
            Node::Link { to, name: _ } => {
                match parse_url(Some(base_url.clone()), to.clone()) {
                    Ok(u) => found.push(u),
                    Err(_) => (), //eprintln!("debug: failed to parse {}: {}", to, e),
                }
            },
            _ => (),
        }
    }

    found
}

fn parse_url<T>(base_u: Option<Url>, u: T) -> Result<Url, Box<dyn Error>>
where
    T: Into<String> + Clone
{
    // try to parse url
    // if it fails because the url is relative, try again using
    // base_u as the base url
    let mut ur = match Url::parse(&u.clone().into()) {
        Ok(u) => u,
        Err(cause) => {
            match cause {
                ParseError::RelativeUrlWithoutBase => {
                    if let Some(base) = base_u {
                        base.join(&u.into())?
                    } else {
                        Err("gave relative url, but no base url")?
                    }
                },
                _ => Err(cause)?,
            }
        },
    };

    if ur.port().is_none() {
        ur.set_port(Some(1965)).unwrap();
    }

    if ur.scheme() != "gemini" {
        Err("invalid url scheme")?;
    }

    Ok(ur)
}

// the following was stolen from Christine Dodrill's majc project
// https://tulpa.dev/cadey/maj
struct NoCertificateVerification {}

impl rustls::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _roots: &rustls::RootCertStore,
        _presented_certs: &[rustls::Certificate],
        _dns_name: webpki::DNSNameRef<'_>,
        _ocsp: &[u8],
    ) -> Result<rustls::ServerCertVerified, rustls::TLSError> {
        Ok(rustls::ServerCertVerified::assertion())
    }
}

async fn get(ur: Url, cfg: tokio_rustls::rustls::ClientConfig)
    -> Result<Vec<u8>, Box<dyn std::error::Error>>
{
    use tokio::io::{AsyncWriteExt, AsyncReadExt};

    let cfg = Arc::new(cfg);
    let host = ur.host_str().unwrap();
    let name_ref = webpki::DNSNameRef::try_from_ascii_str(host)?;
    let config = TlsConnector::from(cfg);

    let sock = TcpStream::connect(&format!("{}:{}", host,
            ur.port().unwrap())).await?;
    let mut tls = config.connect(name_ref, sock).await?;

    let req = format!("{}\r\n", ur.to_string());

    tls.write(req.as_bytes()).await?;
    let mut buf: Vec<u8> = vec![];
    tls.read_to_end(&mut buf).await?;

    Ok(buf)
}
