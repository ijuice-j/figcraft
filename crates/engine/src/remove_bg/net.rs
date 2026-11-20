//! Talking to the user's background remover over HTTPS (desktop): sending an image, and the test
//! the app's settings run. Failures come back as sentences the app shows as they are.

use std::time::{Duration, Instant};

use super::{Connection, MAX_BYTES};

/// How long one removal may take, upload included.
const TIMEOUT: Duration = Duration::from_secs(90);

/// The largest answer taken: a cut-out PNG of a 20 MB photo is well below this.
const MAX_ANSWER: u64 = 256 * 1024 * 1024;

/// An agent that hands back every status (they are told apart here) and gives up after `timeout`.
pub(crate) fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(timeout)).user_agent("FigCraft").build(),
    )
}

/// The media type of an encoded image, from its first bytes.
fn media_type(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Why a request didn't reach the remover, as a sentence.
pub(crate) fn unreachable(e: &ureq::Error, address: &str) -> String {
    match e {
        ureq::Error::Timeout(_) => "The background remover didn't answer in time. Try again.".into(),
        ureq::Error::HostNotFound => {
            format!(
                "FigCraft couldn't find your background remover at {address}. Check the address in Preferences, under AI tools, or your internet connection."
            )
        }
        ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            "FigCraft couldn't reach your background remover. Check your internet connection and try again.".into()
        }
        other => format!("FigCraft couldn't reach your background remover ({other})."),
    }
}

/// Send `image` to the remover; its cut-out PNG, or why not.
pub fn remove(c: &Connection, image: &[u8]) -> Result<Vec<u8>, String> {
    if image.len() > MAX_BYTES {
        return Err("The image is over 20 MB, the most the background remover takes.".into());
    }
    let url = format!("{}/remove-background", c.address);
    let response = agent(TIMEOUT)
        .post(&url)
        .header("x-access-key", &c.key)
        .header("content-type", media_type(image))
        .send(image)
        .map_err(|e| unreachable(&e, &c.address))?;
    let status = response.status().as_u16();
    let body = response.into_body().with_config().limit(MAX_ANSWER).read_to_vec().map_err(|e| unreachable(&e, &c.address))?;
    match status {
        200 if body.starts_with(&[0x89, b'P', b'N', b'G']) => Ok(body),
        200 => Err("The background remover sent back something that isn't an image. Reinstall it in Preferences, under AI tools.".into()),
        401 | 403 => Err("Your background remover didn't accept FigCraft's access key. Connect it again in Preferences, under AI tools.".into()),
        404 => Err(format!(
            "There's no background remover at {}. Check the address in Preferences, under AI tools. A remover installed a moment ago can take a minute to start.",
            c.address
        )),
        413 => Err("The image is over 20 MB, the most the background remover takes.".into()),
        429 => Err("Cloudflare is getting too many requests. Wait a minute and try again.".into()),
        _ => {
            let said = String::from_utf8_lossy(&body).chars().filter(|c| !c.is_control()).take(300).collect::<String>();
            let lower = said.to_lowercase();
            if lower.contains("9422") || lower.contains("limit") || lower.contains("quota") {
                Err("Your Cloudflare account has used this month's free images. It starts again next month.".into())
            } else if said.trim().is_empty() {
                Err(format!("The background remover failed (error {status}). Try again."))
            } else {
                Err(format!("The background remover failed (error {status}): {}", said.trim()))
            }
        }
    }
}

/// A small made-up picture for testing a remover: a round orange subject on a blue backdrop.
pub fn test_image() -> Vec<u8> {
    const W: u32 = 160;
    const H: u32 = 120;
    let Some(mut px) = figcraft_render::Pixmap::new(W, H) else { return Vec::new() };
    for (i, p) in px.data_mut().chunks_exact_mut(4).enumerate() {
        let (x, y) = ((i as u32 % W) as f32, (i as u32 / W) as f32);
        let inside = (x - 80.0).powi(2) + (y - 60.0).powi(2) < 38.0 * 38.0;
        let rgb: [u8; 3] = if inside { [240, 120, 40] } else { [40, 70 + (y as u8 / 2), 150] };
        p.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    figcraft_render::encode_png(&px).unwrap_or_default()
}

/// Send [`test_image`] to the remover: how long it took, or why it failed.
pub fn test(c: &Connection) -> Result<Duration, String> {
    let start = Instant::now();
    remove(c, &test_image()).map(|_| start.elapsed())
}
