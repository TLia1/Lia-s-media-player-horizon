# Lia's Media Player for Horizon

**Pictures, GIFs, videos and music inside Horizon.**

Keep a reference picture beside your build, watch a clip in a floating window,
or listen to a playlist while you play.

**Horizon · Minecraft Java Edition 26.3 · Client-side native mod**

Install it in your Horizon instance. Playback and your saved library are local
to your client, so servers and other players can play as usual.

Rewrite in rust of the original mod in java by me, check it [here](https://modrinth.com/mod/lias-media-player)

## From chat to your screen

Supported media links appear as compact badges such as **[picture]**,
**[youtube]** and **[audio]**.

- **Hover** a badge for a preview when one is available.
- **Click** an image badge to open the picture, or click an individual video
  or audio badge to open a separate player.
- **Shift + click** an individual video or audio badge to queue it in a player
  of the same kind.
- **Ctrl + click** to open the original link in your browser.

Use the **Playlists** and **Hidden** buttons in chat to reach your library or
bring back hidden windows.

## Pictures, video and music

| Media | Supported links |
| --- | --- |
| **Pictures and animations** | PNG, JPEG, BMP, GIF, WebP, animated WebP, AVIF and animated PNG, plus Tenor and Giphy share links |
| **Video and live streams** | YouTube, Twitch, Vimeo, Streamable, Reddit video, direct video files, and HLS or DASH streams |
| **Music** | SoundCloud tracks and sets, Bandcamp tracks and albums, and direct audio files such as MP3, FLAC and OGG |

Move and resize floating windows to fit your screen. Pin media on your HUD, use
theatre mode for video, or hide a player while it keeps playing. Audio opens in
a compact bar that fits beside the game.

Play or pause, seek through recorded media, skip tracks, adjust volume and
speed, and choose a video quality. In the queue, double-click a track to play
it, drag to reorder, or remove it. Shuffle the queue or cycle repeat from
**off → all → one → off**.

## Playlists, history and favourites

- **Build a library.** Create playlists, search for tracks, and add links
  individually or expand a supported collection such as a YouTube playlist or
  SoundCloud set.
- **Share or import.** Copy a playlist as M3U text, or import M3U or media links
  from your clipboard.
- **Listen your way.** Play in order, shuffle, loop, or append a playlist to an
  audio player. Saved playlists play as audio, even when a track links to a video.
- **Find something again.** Reopen recently played media from History or mark
  items as favourites. Your library is saved between sessions.

## Controls

Press **M** to open Media Controls. The workspace leaves the world visible
while you interact with your media windows.

| Default key | Action |
| --- | --- |
| **M** | Open Media Controls |
| **K** | Play or pause |
| **N** | Next track |
| **P** | Open Playlists |
| **H** | Open Media History |

Rebind these keys in **Options → Mods → Lia's Media Player**.

You can also open a link privately from chat with this command.

```text
/show <image|gif|video|audio> <url> [newPlayer]
```

Add `true` at the end to open a separate window. Without it, video and audio
join a compatible player's queue when one exists. Use `/show audio` with a
video link to listen to its soundtrack. For a YouTube video URL with a
`list=` parameter, remove that parameter to play just the video.

## Getting started

1. Build the native mod for your instance through Horizon Launcher, then launch
   that instance.
2. Enable **Lia's Media Player** in **Options → Mods** and enter a world.
3. Click a media badge in chat or try `/show` with a media URL.

In the mod's **Settings**, choose Dark, High Contrast, Minecraft or Light.
You can also toggle chat previews and the playback HUD, set the default video
quality and frame rate, and choose whether hidden video switches to sound only.

FFmpeg decodes audio and video, and yt-dlp opens supported sites and collections.
The mod can use installed tools or download missing ones on demand when enabled.
If playback fails, open the player's **…** menu for details or check
**Settings → Tools**.
