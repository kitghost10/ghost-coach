# Ghost Coach

Ghost Coach is a desktop call coach. It captures your microphone and the prospect's call audio, shows live captions, and can suggest a response. It does not speak on the call.

## Linux setup (Ubuntu and Debian)

1. Install the desktop build dependencies:

   ~~~bash
   ./setup_linux.sh
   ~~~

   Linux audio capture uses ALSA for the microphone and a PulseAudio monitor source for system audio. PipeWire works when its PulseAudio compatibility service is enabled. The pulseaudio-utils package provides the pactl and parec commands used to find and read the monitor source.

   On Wayland, the launcher uses XWayland when it is available so the coach can honor its placement above the call. To force the native Wayland backend, run GDK_BACKEND=wayland ./start.sh; the compositor may then ignore the overlay's requested position.

2. Build and update the local launcher binary:

   ~~~bash
   ./rebuild.sh
   ~~~

3. Launch Ghost Coach:

   ~~~bash
   ./start.sh
   ~~~

   The Applications menu shortcut also launches this local binary.

For frontend development, use npm run app; Tauri starts the Vite server on 127.0.0.1:14200. npm run desktop builds a release Linux executable with the frontend embedded.

## Windows

Install Node.js and Rust, then run npm.cmd install and npm.cmd run tauri -- dev from PowerShell. If PowerShell blocks npm, use the .cmd command shown.

## First call

Use a headset, add a Deepgram API key in Setup, verify the microphone and system-audio meters, then start a session. An OpenRouter API key and a manually entered model ID are optional; without one, captions still work but reply suggestions are disabled. Stop the session when finished.

Deepgram processes live audio while a session is running. Saved transcripts and audio remain in the local application data folder.

## Core workflow

1. **Prepare:** enter a prospect name, call goal, and short brief.
2. **Coach:** start coaching, then use your calling app as usual. Read captions and suggested replies in the main window or floating coach.
3. **Save:** end the session to save the recording and transcript.
4. **Review:** open Call library to listen to the recording or copy the transcript.

Connections contains a Deepgram API key, an OpenRouter API key, and a blank model field for entering the full OpenRouter model ID. Floating-coach preferences are limited to light/dark theme and text size. Older configuration files load automatically; retired overlay preferences are dropped when settings are saved.

## Floating coach troubleshooting

Use **Preview floating coach** on New call to show the overlay without starting a recording or connecting to a provider. The same preview is available with `./start.sh --preview-coach`. Starting coaching raises the floating window on the same screen as the main app. If audio or transcription fails, the coach remains visible with the error; fix the connection or audio setup in the main window and start again.
