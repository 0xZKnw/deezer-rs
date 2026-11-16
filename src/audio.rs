//! Lecture audio via rodio. Un `Player` par morceau (créé/détruit à la volée) : zéro état résiduel.

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::{io::Cursor, sync::Arc, time::Duration};

pub struct Audio {
    sink: MixerDeviceSink,
    player: Option<Player>,
    volume: f32,
    pub len: Duration,
}

/// Wrapper permettant de partager le buffer MP3 (Arc) sans le copier.
struct SharedBytes(Arc<Vec<u8>>);
impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Audio {
    pub fn new(volume: f32) -> Result<Self, String> {
        let sink = DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?;
        Ok(Self { sink, player: None, volume, len: Duration::ZERO })
    }

    pub fn play(&mut self, bytes: Arc<Vec<u8>>) -> Result<(), String> {
        self.stop();
        let len = bytes.len() as u64;
        let dec = Decoder::builder()
            .with_data(Cursor::new(SharedBytes(bytes)))
            .with_byte_len(len)
            .with_seekable(true)
            .build()
            .map_err(|e| e.to_string())?;
        self.len = dec.total_duration().unwrap_or(Duration::from_secs(30));
        let p = Player::connect_new(self.sink.mixer());
        p.set_volume(self.volume);
        p.append(dec);
        p.play();
        self.player = Some(p);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(p) = self.player.take() {
            p.stop();
        }
    }

    pub fn has_player(&self) -> bool {
        self.player.is_some()
    }

    pub fn toggle(&self) {
        if let Some(p) = &self.player {
            if p.is_paused() {
                p.play()
            } else {
                p.pause()
            }
        }
    }

    pub fn paused(&self) -> bool {
        self.player.as_ref().map(|p| p.is_paused()).unwrap_or(true)
    }

    pub fn pos(&self) -> Duration {
        self.player.as_ref().map(|p| p.get_pos()).unwrap_or_default()
    }

    pub fn seek(&self, to: Duration) {
        if let Some(p) = &self.player {
            let _ = p.try_seek(to);
        }
    }

    pub fn finished(&self) -> bool {
        self.player.as_ref().map(|p| p.empty()).unwrap_or(false)
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v;
        if let Some(p) = &self.player {
            p.set_volume(v);
        }
    }
}

/// Retire l'en-tête ID3v2 (les previews Deezer en ont un de taille 0 que symphonia refuse).
pub fn clean_mp3(mut v: Vec<u8>) -> Vec<u8> {
    if v.len() > 10 && &v[..3] == b"ID3" {
        let size = ((v[6] as usize & 0x7f) << 21) | ((v[7] as usize & 0x7f) << 14) | ((v[8] as usize & 0x7f) << 7) | (v[9] as usize & 0x7f);
        let footer = if v[5] & 0x10 != 0 { 10 } else { 0 };
        let cut = (10 + size + footer).min(v.len());
        v.drain(..cut);
    }
    v
}
