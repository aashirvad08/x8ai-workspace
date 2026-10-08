//! What `x8ai` in a terminal and the background `x8ai` say to each other over
//! their socket (ADR 0024): keys, the mouse, pastes and the terminal's size
//! one way; what to draw, and when to stop, the other.
//!
//! A frame is a kind (1 byte), a length (4 bytes, big-endian) and that many
//! bytes. What to draw goes as it is; everything else is JSON, so the first
//! frame each way stays readable by any version, and a client and a background
//! `x8ai` of different protocols can say so instead of misreading each other.

use std::io::{self, Read, Write};
use std::path::PathBuf;

use crossterm::event::Event;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Changes whenever a frame's meaning does.
pub const PROTOCOL: u32 = 1;

/// The most one frame may hold: far more than a screen's worth of drawing.
const MAX_FRAME: usize = 16 << 20;

const JSON: u8 = 1;
const DRAW: u8 = 2;

/// The first thing a terminal says: who it is, and what it has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub version: String,
    /// Where `x8ai` was started: relative folders are found from here.
    pub cwd: PathBuf,
    /// `x8ai <folder>`.
    pub folder: Option<String>,
    /// Columns and rows.
    pub size: (u16, u16),
    /// Whether the terminal shows 24-bit color.
    pub truecolor: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToServer {
    Hello(Hello),
    Input(Event),
}

/// The terminal is to give itself back to the shell, show this (an error when
/// `code` is not 0) and end with `code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exit {
    pub message: Option<String>,
    pub code: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToClient {
    /// Bytes for the terminal: escape sequences and text.
    Draw(Vec<u8>),
    Exit(Exit),
}

fn frame(kind: u8, payload: &[u8]) -> io::Result<Vec<u8>> {
    let length = u32::try_from(payload.len())
        .ok()
        .filter(|&n| n as usize <= MAX_FRAME)
        .ok_or_else(|| io::Error::other("a frame is too large"))?;
    let mut bytes = Vec::with_capacity(5 + payload.len());
    bytes.push(kind);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

fn json<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    frame(JSON, &serde_json::to_vec(value).map_err(io::Error::other)?)
}

/// Reads one frame; `None` at the end of the stream.
fn read_frame(reader: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut header = [0u8; 5];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if length > MAX_FRAME {
        return Err(io::Error::other("a frame is too large"));
    }
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(Some((header[0], payload)))
}

fn parse<T: DeserializeOwned>(kind: u8, payload: &[u8]) -> io::Result<T> {
    if kind != JSON {
        return Err(io::Error::other("an unexpected frame"));
    }
    serde_json::from_slice(payload).map_err(io::Error::other)
}

pub fn send(writer: &mut impl Write, message: &ToServer) -> io::Result<()> {
    writer.write_all(&json(message)?)?;
    writer.flush()
}

pub fn send_to_client(writer: &mut impl Write, message: &ToClient) -> io::Result<()> {
    let bytes = match message {
        ToClient::Draw(bytes) => frame(DRAW, bytes)?,
        ToClient::Exit(exit) => json(exit)?,
    };
    writer.write_all(&bytes)?;
    writer.flush()
}

pub fn receive(reader: &mut impl Read) -> io::Result<Option<ToServer>> {
    read_frame(reader)?
        .map(|(kind, payload)| parse(kind, &payload))
        .transpose()
}

pub fn receive_from_server(reader: &mut impl Read) -> io::Result<Option<ToClient>> {
    let Some((kind, payload)) = read_frame(reader)? else {
        return Ok(None);
    };
    if kind == DRAW {
        return Ok(Some(ToClient::Draw(payload)));
    }
    parse(kind, &payload).map(|exit| Some(ToClient::Exit(exit)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn messages_arrive_as_sent() {
        let hello = ToServer::Hello(Hello {
            protocol: PROTOCOL,
            version: "1.2.3".to_owned(),
            cwd: PathBuf::from("/Users/me/code"),
            folder: Some("app".to_owned()),
            size: (100, 30),
            truecolor: true,
        });
        let key = ToServer::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));
        let mut bytes = Vec::new();
        send(&mut bytes, &hello).unwrap();
        send(&mut bytes, &key).unwrap();
        let mut reader = bytes.as_slice();
        assert_eq!(receive(&mut reader).unwrap(), Some(hello));
        assert_eq!(receive(&mut reader).unwrap(), Some(key));
        assert_eq!(receive(&mut reader).unwrap(), None);

        let draw = ToClient::Draw(b"\x1b[2J hello".to_vec());
        let exit = ToClient::Exit(Exit {
            message: Some("bye".to_owned()),
            code: 0,
        });
        let mut bytes = Vec::new();
        send_to_client(&mut bytes, &draw).unwrap();
        send_to_client(&mut bytes, &exit).unwrap();
        let mut reader = bytes.as_slice();
        assert_eq!(receive_from_server(&mut reader).unwrap(), Some(draw));
        assert_eq!(receive_from_server(&mut reader).unwrap(), Some(exit));
        assert_eq!(receive_from_server(&mut reader).unwrap(), None);
    }

    #[test]
    fn a_frame_too_large_or_cut_short_is_refused() {
        let mut huge = vec![JSON];
        huge.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(receive(&mut huge.as_slice()).is_err());
        let mut cut = Vec::new();
        send(&mut cut, &ToServer::Input(Event::FocusGained)).unwrap();
        cut.truncate(cut.len() - 1);
        assert!(receive(&mut cut.as_slice()).is_err());
        // Drawing is not something a terminal sends.
        let draw = frame(DRAW, b"x").unwrap();
        assert!(receive(&mut draw.as_slice()).is_err());
    }
}
