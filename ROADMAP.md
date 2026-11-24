# Roadmap

## M0 — the first version (0.1, this one)

- [x] The recording: microphone and computer sound, Opus in five-minute files, silence left out,
      kept 48 hours (a setting), read while it's written
- [x] The computer's sound without RecallCraft's own (process loopback), the default speaker's
      loopback where that isn't there
- [x] Calls found by which program holds a microphone open (WASAPI audio sessions), webviews put
      down to their app; the meeting's title from its window; browser meetings named by service
- [x] The call's microphone followed during a call; a Bluetooth headset's only during calls
- [x] Live transcript: utterances cut at pauses, packed into Whisper windows, heard in a worker
      program; what was said just before a call was noticed is heard too
- [x] You and the others by stream; the others told apart by voice (cepstra and pitch,
      Bhattacharyya distance), named voices known again on later calls
- [x] Echo: the speakers heard by the microphone left out, by loudness and by words
- [x] Tray: Replay, Pause, Settings, status; Replay shortcut; a dot during calls
- [x] Window: calls by day, live call, transcript following playback, speed 0.75–2× (pitch kept),
      activity strip, catch up and live, rename, keep, delete, copy, search, last 48 hours by minute
- [x] Export: WAV mixed/stereo/separate, Opus; text, Markdown, SRT, WebVTT, JSON
- [x] Sign-in service with a supervisor; one instance; logs; CLI

## M1 — next

- [ ] An installer (MSI, per user), with the model beside the program; a Start menu entry
- [ ] Names from the meeting app itself: the participant list and the live captions' speakers
      through UI Automation (Teams, Zoom), matched to the voices by time
- [ ] Encryption at rest (Windows' data protection for the user), keys per file
- [ ] A storage limit as well as a time limit
- [ ] Transcribe any stretch of the last two days on request (not only calls)
- [ ] Re-hear a call with a larger model (`small.en`) when the computer is idle
- [ ] Mark a moment during a call (a shortcut), shown in its transcript
- [ ] Edit a line of the transcript

## Later

- [ ] macOS: the computer's sound (ScreenCaptureKit audio), calls by the microphone in use
- [ ] Linux: the computer's sound (PipeWire monitor), calls by the microphone in use
- [ ] Other languages (the multilingual Whisper models need the language told)
- [ ] Summaries of a call, on this computer
