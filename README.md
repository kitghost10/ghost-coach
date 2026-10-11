# Ghost Coach

Ghost Coach is a desktop call coach. It captures your microphone and the prospect's call audio, shows live captions, and can suggest a response. It does not speak on the call.

## Linux setup (Ubuntu and Debian)

1. Install the desktop build dependencies:

   ~~~bash
   ./setup_linux.sh
   ~~~

   Linux audio capture uses ALSA for the microphone and a PulseAudio monitor source for system audio. PipeWire works when its PulseAudio compatibility service is enabled. The pulseaudio-utils package provides the pactl and parec commands used to find and read the monitor source.

   On Wayland, the launcher uses XWayland when it is available so the coach can honor its top/bottom placement. To force the native Wayland backend, run GDK_BACKEND=wayland ./start.sh; the compositor may then ignore the overlay's requested position.

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

Use a headset, add a Deepgram API key in Settings, verify the microphone and system-audio meters, then start a session. An AI provider key is optional; without one, captions still work but reply suggestions are disabled. Stop the session when finished.

Deepgram processes live audio while a session is running. Saved transcripts and audio remain in the local application data folder.
