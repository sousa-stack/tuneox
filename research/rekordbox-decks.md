# rekordbox 7 decks: DUAL PLAYER, transport, sync and waveforms

What rekordbox 7.2.19 (macOS) does in its EXPORT-mode players, so TuneOx's dual player can feel the same. Part A of [docs/specs/decks.md](../docs/specs/decks.md).

Sources, cited inline with short tags:

- **[M p.N]**: rekordbox 7.2.18 instruction manual (EN), <https://cdn.rekordbox.com/files/20260807093645/rekordbox7.2.18_manual_EN.pdf>. Page numbers are the printed ones.
- **[KB]**: "Default keyboard shortcut reference" for rekordbox 7.0.5, <https://cdn.rekordbox.com/files/20241203185020/rekordbox7.0.5_default_keyboard_shortcut_reference_EN.pdf>.
- **[BIN]**: the rekordbox 7.2.19 arm64 binary. It still has its local symbol table, so functions are cited by name and address. Constants were read from disassembly. Targeted functions were decompiled with Ghidra 12.1.4 headless without auto-analysis. Raw output stayed in the session scratchpad, not the repo.
- **[RES]**: files in the app bundle (`Contents/Resources/skins/*`), read for colour values only.
- **[USR]**: the maintainer's own rekordbox settings and key map (`~/Library/Application Support/Pioneer/rekordbox6/rekordbox3.settings` and `KeyMappings/*.mappings`), read-only.
- **[DB]**: the read-only copy of the maintainer's library, `.re/fixtures/master-7.2.19.db`.
- **[ANLZ]**: the four ANLZ sets in `.re/fixtures/usb-tuneox-test-7.2.19/`.
- **[DJL]**: Deep Symmetry's DJ Link analysis, <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html>.
- **[BL]**: beat-link (`CueList`, `WaveformDetail`, `WaveformPreview`, `WaveformFinder`, `*Component`), <https://github.com/Deep-Symmetry/beat-link>.

Confidence notes: **(high)** means read directly from code, data or the manual. **(medium)** means inferred from code whose context is incomplete. **(low)** means a guess, listed in the open questions.

---

## 1. DUAL PLAYER (EXPORT mode) and the preview player

### 1.1 Layouts

EXPORT mode has five player layouts: 1 PLAYER, 2 PLAYER, DUAL PLAYER, SIMPLE PLAYER and FULL BROWSER [M p.62–64, p.254].

- **DUAL PLAYER** is 2 PLAYER plus extras: per its tooltip, 3-band EQ (kills), KEY SYNC and DUAL CONTROL [BIN strings]. It needs a paid plan, a hardware unlock or owner registration [M p.122]. The binary says "higher than Core Plan".
- Built-in shortcuts: ⌘7 = 1 Player, ⌘8 = 2 Players, ⌘9 = Simple Player, ⌘0 = Full Browser [USR Export preset]. DUAL PLAYER has no default shortcut; it is picked from the layout menu.

### 1.2 Screen layout of DUAL PLAYER

Read from the manual screenshots [M p.63, 65, 67, 69] (the 1263×459 screenshot is the reference):

```
┌ op ─┬ mix ─┬──────────────── Deck A ─────────────────────────────┬─ sub ─┐
│ <  >│-0.0dB│ [art] Title / Artist     BPM key   -02:54.4  04:55.9 │A 00:01│
│4Beats│A-GAIN│                          KEY SYNC [BEAT SYNC]       │B 00:00│
│(CUE)│ HIGH │                          < Bbm ±0 >  [MASTER]        │C ...  │
│ (▶) │ MID  │ ▔▔▔ overview (hot cue letters above, phrase bar below)│  ...  │
│     │ LOW  │ |||◄ ! ►||| MEMORY AT MT < 8 >  − 119.89 +  ◄◄ ►►  MT RST Q│MEM/HOT│
│ [⧉] │  A   │ ───── Deck A detail waveform (half, grows upward) ───│ /INFO │
│     │  ┃   │ ─ phrase bars A │ playhead │ phrase bars B ─        ├───────┤
│ <  >│  ┃ X │ ───── Deck B detail waveform (half, grows downward) ─│A 00:03│
│4Beats│  B  │ [art] Title / Artist ...  (same as deck A)          │B 01:22│
│(CUE)│ A-GAIN│ overview, function panel                            │  ...  │
│ (▶) │ H/M/L │                                                     │       │
└─────┴──────┴──────────────────────────────────────────────────────┴───────┘
```

- Deck A sits on top and deck B below. **The two enlarged waveforms are stacked between the decks and face each other.** Deck A's half-waveform grows upward from its baseline and deck B's grows downward. Each deck's phrase bar runs along its baseline, and both share one center playhead line, so beats line up visually [M p.63, p.69 images] **(high)**. In 1 PLAYER the enlarged waveform is a full, mirrored waveform instead.
- The bar counter (`148.1Bars`, `62.3Bars`, `-1.4Bars`) is drawn in accent blue right above (deck A) or below (deck B) the playhead. The format is bar.beat-in-bar, negative before the first downbeat. The [Beat Count Display] preference switches it to "count to next memory cue (bars/beats)" [M p.92] **(high)**.
- The left edge of each enlarged waveform has zoom − / [RST] / + and a hide toggle. The right edge has the **Matching** button: a link icon that saves the A/B pair as a good match for RELATED TRACKS, and lights blue if the pair is already saved [M p.75, p.121].

### 1.3 Every per-deck control

**Operation panel** (left column, one block per deck) [M p.67–68, BIN `djplay::PlayCuePanel`]:

| Control | Behaviour |
| --- | --- |
| `<` `>` | Beat jump back/forward by the selected size. During a loop it moves the loop (in and out) [M p.90]. |
| Size dropdown (`cbSelectBeat`) | Items `Fine`, `4Beats`, `8Beats`, `16Beats`, `8Bars`, `16Bars`, `32Bars` (ids 1–7). The default is id 2 = **4Beats** (`SettingIF::getExportBeatJump` default 2) [BIN `PlayCuePanel` ctor 0x100008294] **(high)**. |
| CUE | Round button with a #FFB937 ring [RES `cueBtn_on_.svg`]. Semantics in §2.1. |
| PLAY/PAUSE | Round button with a grey ring. |
| DUAL CONTROL (⧉, between the decks) | Links both players. Its tooltip says it links the waveform controls and beat jump of both players, and the manual says PLAY, CUE and beat jump act on both decks [M p.121, BIN strings]. The icon turns accent blue #1473EB when on [RES `dualControllOn.svg`]. |

**Mixer column** (DUAL PLAYER only) [M p.68, p.122]:

| Control | Behaviour |
| --- | --- |
| A-GAIN knob plus a dB readout | Auto Gain per deck. It loads the gain computed during analysis and can be adjusted by hand; **the adjustment is saved with the track**. The readout format is `+%2.1fdB` or `%2.1fdB` (one decimal) [BIN strings]. The slider range looks like −10…+10 dB (`AutoGainSlider` ctor arg) **(medium)**. The stored gain lives in `djmdMixerParam.GainHigh/GainLow`, a float32 split into two u16 halves that holds **linear** gain; real values range from about −1.4 dB to +14.4 dB. `PeakHigh/PeakLow` uses the same split for peak amplitude [DB] **(high)**. |
| HIGH / MID / LOW | **Kill toggles, not knobs**: each mutes one band and is not saved with the track [M p.122]. In the engine each band is normalised so that 0.5 is flat and 0 is a kill (`PlayCuePanel::buttonClicked` toggles between 0 and 0.5) [BIN] **(high)**. Off: grey #282828 pill with a white label. On: the same pill with an #1473EB outline and label [RES `eqHighOn.svg`]. |
| Vertical crossfader, A at the top and B at the bottom | Range −10…+10, continuous; double-click returns it to centre. It drives the engine crossfader as a 0…1 value [BIN `PlayCuePanel` ctor, `updateSliderValue` → `DjEngineIF::setCrossFader`] **(high)**. The curve is the engine's crossfader curve; the EXPORT-mode curve is not known (open question). |

**No channel faders, no EQ knobs, no trim, no headphone/PFL, no FX, no sampler, no slip, no jog and no tempo slider.** There is one stereo output. DJs audition a transition by playing both decks (or starting them together with DUAL CONTROL), syncing, riding the crossfader, killing the lows on the incoming track and swapping bass at the drop with the LOW kills, and matching loudness with A-GAIN. All of this happens on one master output; there is no cueing in headphones [M p.119–122, p.235: EXPORT mode has a single output-channel setting] **(high)**.

**Track information panel** (per deck) [M p.69–70]:

- Artwork; title in white bold with the artist below; BPM (two decimals, e.g. `119.89`) and key (e.g. `Bbm`, format set in preferences).
- **Remaining time** large (`-02:54.4`, with tenths) and **elapsed** grey (`04:55.9`). The T key toggles time mode [USR].
- KEY SYNC button; key shift `<` key ±N `>` (semitones); KEY RESET (it appears in the function panel when the key has been shifted).
- BEAT SYNC (accent blue when on) and MASTER (outlined; filled when this deck is master).
- Overview ("full waveform"). Hot cue letter squares sit **above** it, memory cue markers on top, and the current cue as an orange up-triangle at the **bottom-left**. Phrase bars (phrase analysis) sit below, and vocal-detection marks appear as cyan segments above when enabled [M p.69 image, p.233].

**Function panel** (per deck, DUAL layout) [M p.79, image p.79]:

`|||◄  !  ►|||` (beat grid adjust) · `MEMORY` (save the current cue or loop as a memory cue) · `AT`/`MT` (switches between auto beat loop and manual loop) · `< 4 >` (auto beat loop size, or IN / OUT / RELOOP-EXIT in manual mode) · `− 119.89 +` (tempo) · `◄◄ ►►` (pitch bend, hold) · `MT` (master tempo) · `RST` (tempo reset) · `Q` (quantize, **per deck**, stored as `Player%d_Quantize`) [BIN `PlayerControllPanel::buttonClicked`].

**Sub-information panel** (right column, one per deck): tabs MEMORY / HOT CUE / INFO [M p.71–74].

- HOT CUE lists slots A–H with their time and comment. Clicking a saved slot jumps to it and plays. **Clicking an empty slot stores the current position** (or the current loop as a hot loop).
- MEMORY lists up to 10 memory cues/loops with min:sec.ms, colour, comment and an active-loop toggle. Clicking one jumps to it and **pauses**.

### 1.4 Loading tracks

- Drag a track from the browser onto a deck [M p.90]. Keyboard: Performance presets use ⇧← / ⇧→ (load to deck 1/2) [KB]. The EXPORT preset has **no load keys** [USR].
- Dragging the title area of one loaded deck onto the other deck duplicates it (Instant Doubles) [M p.169].
- On load in EXPORT mode the cue is **not** moved to a memory cue. `UiPlayer::setCurrentCueAfterLoad` only looks for the earliest memory or hot cue when the app is in the mode where the "Load Setting" preference applies (PERFORMANCE). Otherwise the cue stays unset and the deck parks at the start [BIN 0x101ba0624, `findFirstCuePoint` 0x101ba08ec] **(medium)**. Loading a different track clears the previous cue and loop [M p.97, p.99].
- Analysed tracks only: SYNC needs a beat grid, and rekordbox shows an error on tracks without one [BIN strings].

### 1.5 The preview player (browser)

[M p.90–91]:

- The **Preview** column shows a waveform thumbnail. Clicking it plays from the clicked position, and a stop button sits to its left. Previewing **pauses any deck that is playing**.
- **Artwork column**: hovering previews from the start of the track and moving the mouse away stops it. Each click skips ahead 30 s.
- Optionally ([Display Cue Markers on Preview]) the thumbnail shows memory and hot cue markers, and clicking near one starts the preview there.
- Shortcuts: ⌘P preview play/stop, ⌘[ preview skip [USR Export preset].
- The thumbnail is the same 1200-point overview image the deck uses (`WavPointCache::getWholeWavePoint` → `load_wholeWave` / `load_3bandsWholeWave`) [BIN] **(high)**.

### 1.6 What EXPORT mode lacks compared with PERFORMANCE mode

| | EXPORT DUAL PLAYER | PERFORMANCE 2-deck |
| --- | --- | --- |
| Mixer | crossfader, HIGH/MID/LOW **kills**, Auto Gain | full mixer: channel faders, 3-band EQ/ISOLATOR knobs, trim, crossfader curves, headphone cue, FX |
| Tempo | −/+ buttons, BPM entry, RST; no slider, no range selector | slider plus ±6/10/16/WIDE range, −/+ on the platter |
| Pads | none (hot cues via the sub panel or keys) | HOT CUE A–P, BEAT JUMP, BEAT LOOP, SLICER, PAD FX… |
| Slip, reverse, jog, DVS | no | yes |
| Hot cues | A–H | A–P |

---

## 2. Transport behaviour

### 2.1 CUE (rekordbox = CDJ convention)

[M p.97, p.160, BIN tooltip strings] **(high)**:

1. **Paused, CUE pressed** → the current position becomes the cue point, and a cue marker appears at the **bottom** of the waveform. If Q is on, the cue snaps to the beat grid; the manual warns it may land slightly away from where you pressed [M p.160]. Setting a new cue replaces the old one.
2. **Playing, CUE pressed** → *Back Cue*: jump to the cue point and pause.
3. **Paused at the cue, CUE held** → *Cue Point Sampler*: play while held, then return to the cue and pause on release.
4. **While holding CUE, press PLAY** → playback continues after CUE is released (CUE button tooltip in [BIN strings]).
5. **IN during playback** (manual loop section) sets the cue on the fly (*Real-Time Cue*); with Q on it snaps to the nearest beat [M p.97].

There is one cue per deck, and it is lost when another track is loaded.

### 2.2 PLAY/PAUSE

A toggle. When paused away from the cue point, the cue stays where it is; pressing CUE then sets a new one (rule 1).

### 2.3 Hot cues A–H

[M p.101–102, p.164–165, p.248] **(high)**:

- **Empty slot pressed (playing or paused)** → stores the current position. With Q on it snaps to the nearest beat. If a loop is playing, the loop is stored as a **hot loop** (shown orange in the CDJ colour scheme).
- **Saved slot pressed, default setting** → **jump and play**. This applies when paused as well: the manual's recall step says playback starts from the chosen hot cue. Pressing a hot loop jumps to the loop start and loops.
- **Optional "Gate playback during Pause (Gate Cue)"** (off by default): when paused, the hot cue plays only while held, and on release it returns to the hot cue and pauses. It does not apply to a hot cue that holds an Auto Beat Loop.
- **Optional "Smart Cue"** (off by default; `SmartCueEnabled=0` [USR]): pressing a hot cue also moves the current cue there, and the cue marker and CUE button take the hot cue's colour.
- Delete: hover and click ✕ in the HOT CUE panel. Keys: ⌘1/2/3 (deck A) and ⌘⇧1… in Performance [KB, USR].
- **Quantize and hot cue triggers while playing** (PERFORMANCE preference "Mode" under QUANTIZE): when checked, the jump happens immediately and the landing position is shifted so the beat phase is kept. When unchecked, it waits for the next beat [M p.245]. The default is not known; `getDeckQuantizeMode` defaults to 2 [BIN] (open question). EXPORT has no such preference. Assume the immediate, phase-keeping jump **(low)**.

### 2.4 Memory cues and loops

[M p.98, p.100–101] **(high)**:

- Up to **10** per track (cues and loops together). MEMORY saves the current cue, or the current loop during or just after loop play.
- **Call** (click in the MEMORY list, or call next/previous) → jumps to the point and **pauses**. PLAY starts from there, and a memory loop starts looping. Keys in EXPORT: N / B = call next / previous, A S D F G H J K L ; = call memory cue 1–10, X = delete, M = set memory cue [USR].
- Shown at the **top** of the waveform as small **downward triangles** in the cue colour. A memory loop is drawn as a bar from in to out, yellow per the manual [M p.101].
- **Active loop**: a saved loop can be flagged active (icon turns red). Players with the feature, and rekordbox when [Active Loop Playback] is On in the 1 PLAYER menu, automatically engage the loop when playback reaches it.

### 2.5 Quantize

- EXPORT: a **Q button per deck**, persisted per player (`Player%d_Quantize`) [BIN]. PERFORMANCE has a preference to toggle all decks at once [M p.159, p.245]. The manual text says [Q] lights red, but the 7.x screenshots in the same manual draw it in accent blue #1473EB [M p.95, p.65 screenshots].
- **What snaps** with Q on: setting the cue (CUE while paused, IN), loop in/out, storing a hot cue, the hot cue trigger landing position, auto beat loop start, and beat jump (always whole beats) [M p.95–102].
- **Resolution**: "Quantize Beat Value" in EXPORT preferences [M p.248], with choices 1/8, 1/4, 1/2 and 1 beat [BIN strings near `QUANTIZE BEAT VALUE`]. PERFORMANCE has separate values for HOT CUE and LOOP; fractional values (1/16–1/2) disable BEAT SYNC [M p.159]. The default looks like **1 beat** (`getCurrentDeckQuantize` default 5, read as the 5th of 1/16, 1/8, 1/4, 1/2, 1) **(medium)**.

### 2.6 Beat jump

- EXPORT sizes: **Fine, 4 beats, 8 beats, 16 beats, 8 bars, 16 bars, 32 bars**, default 4 beats [BIN].
- PERFORMANCE BEAT JUMP pad sizes: FINE (= 5 ms), 1/8, 1/4, 1/2, 1, 2, 4, 8, 16, 32, 64 and 128 beats [M p.169].
- During a loop, a beat jump moves the loop (in and out) by the same amount [M p.90, p.135].
- EXPORT keys: ← / → jump reverse/forward on deck A and ⇧← / ⇧→ on deck B [USR].

### 2.7 Loops

[M p.99–100, p.162–163] **(high)**:

- **Auto beat loop**: lengths from **1/64 to 512 beats**. Pressing it sets loop-in at the current position (snapped to the beat with Q) and loops the selected number of beats. **Pressing the auto-loop button again exits.** `<` / `>` halve/double the length, anchored at loop-in.
  - EXPORT keys: 4–9 → 1/2/4/8/16/32-beat loop, `/` → loop ÷2, ⌥¥ → loop ×2 (JIS yen key), plus ⇧ for deck B [USR].
  - Default length shown: 8 (2 PLAYER) or 4 (DUAL) in the screenshots.
- **Manual loop**: IN (also sets the cue), OUT starts looping from IN, EXIT/RELOOP.
  - **Exit**: playback continues past loop-out.
  - **Reloop** (during playback): jumps back to the last loop-in and resumes the loop.
  - With Q on, IN and OUT snap to the grid. Setting a new loop replaces the old one.
  - EXPORT keys: I = in, O = out, R = exit/reloop [USR].
- **Hot loop**: a hot cue stored during a loop (§2.3).
- **Memory loop / active loop**: see §2.4.
- **Loop overlay on the detail waveform**: translucent #FFA300 at 50% alpha for the active loop. A blue variant (#40A0FF at 50%) is used for one special loop state, and a faint #E8F0FF at 20% for a stored or inactive loop [BIN `WidgetWaveView::drawLoopAreaGL`, colours from the `DeviceDefs` table built in `__GLOBAL__sub_I_djplay_WidgetWaveView2.cpp`] **(medium: which state maps to which colour)**.

### 2.8 Needle search and waveform mouse actions

- **Overview**: click → jump to that position. Playback state is kept: playing continues, paused stays paused [M p.129]. PERFORMANCE has a "Needle Lock" preference that blocks this while playing [M p.240].
- **Enlarged waveform** (EXPORT): click moves the playhead there, and dragging left/right moves it like scratching [M p.75, p.90].
- An optional preference makes left-click = PLAY/PAUSE and right-click = CUE [M p.91, p.232]. It is disabled by default (`WaveClickPlayEnabled=0`) [USR].

### 2.9 Slip

PERFORMANCE only (SLIP button on the JOG panel; it covers hot cue hold, scratch, pause, loops and reverse) [M p.167]. **EXPORT has no slip.** Out of scope for the dual player.

---

## 3. Tempo and sync

### 3.1 Tempo control in EXPORT

[BIN `PlayerControllPanel::buttonClicked` 0x100ce30e8, M p.120] **(high)**:

- **−/+ buttons**: each click changes tempo by **0.02 %**. Holding a button auto-repeats, and once it has been held **≥ 500 ms** each step is **0.1 %** (`juce::Button::getMillisecondsSinceButtonDown() < 500` selects the fine step). The new BPM must stay in range (`BeatGridAdjustment::isBpmNotOutOfRange`); out-of-range results are clamped to **40–499 BPM** and turned back into a percent.
- **BPM field**: double-click to type a BPM; the value must lie within the tempo range, or rekordbox shows an error [BIN strings].
- **RST** returns to 0 %.
- **Pitch bend** `◄◄` / `►►`: while held and playing, the deck gets a temporary speed offset through the engine's jog-ring nudge (`setJogRingSpeed(±0.1)`), released on button-up. The percent this equals is not known **(medium)**.
- **MT** toggles master tempo (key lock). It is **off by default on every deck** (`kMasterTempoStatusDefaultValue` = 0,0,0,0) [BIN].
- EXPORT keys: F6/F7 = BPM −/+, F4/F5 = pitch bend −/+, F3 = tempo reset, F2 = master tempo, F1 = SYNC; add ⇧ for deck B [USR].

### 3.2 Tempo slider ranges (PERFORMANCE; for the spec's range concept)

- Ranges are ±6, ±10, ±16 % and WIDE [BIN `@TempoRange.6/10/16/WIDE`].
- Step per −/+ click is **0.02 % at ±6, 0.04 % at ±10 and ±16, and 0.5 % at WIDE** [M p.155].
- WIDE is ±100 %; at −100 % playback stops [M p.155].
- Double-clicking the rate % resets it.

### 3.3 Display

BPM is shown with **two decimals** everywhere in EXPORT (`119.89`) [M screenshots]. The key display follows the tempo when MT is off [M p.130].

### 3.4 Sync

[M p.121, p.168, p.248, BIN strings] **(high unless noted)**:

- **Type**: preference "Sync Type" = **BEAT SYNC** (tempo and phase) or **BPM SYNC** (tempo only). The default is BEAT SYNC (`getBeatSyncType` default 1) [BIN] **(medium on the 1 = beat mapping)**.
- **Double/half**: "Allow Beat/BPM Sync with double/half BPM" makes a deck sync at ×2 or ×½ when that is closer to its own BPM. PERFORMANCE adds a SYNC RATE button (×1 → ×2 → ×½, skipping ×2 if it would exceed +100 %) [M p.168–169].
- **Master selection**:
  - **The player that starts playing first becomes the Sync Master** [M p.121].
  - Pressing MASTER on the other deck hands master over.
  - **When the master's track is changed or unloaded, master passes to the other deck** [M p.168].
  - SYNC on the master deck itself stays lit but has nothing to follow.
- **Slave restrictions**: while sync is on, **only the master's tempo can be changed or reset**. On a synced slave, rekordbox rejects tempo-slider moves and tempo resets with "invalid operation" messages saying they only work on the master deck [BIN strings]. When the master's tempo changes, slaves follow.
- **Nudging a synced deck**: pitch bend on a beat-synced deck shifts its phase. Sync then **degrades to BPM sync** and the SYNC button turns **yellow/orange**. The button is also yellow/orange whenever BPM SYNC is the chosen type. Turning SYNC off and on restores beat sync; the binary has a status message telling the user exactly that [BIN strings].
- **Turning SYNC off** keeps the synced tempo. RST then returns to the track's original BPM [M p.121].
- **Loops and hot cues with sync**: with BEAT SYNC and Q both on, hot cues and loops keep the rhythm aligned [M p.159]. A manual loop set with Q off, or reverse play, breaks beat sync [M p.218, DRUM SWAP notes].

---

## 4. Waveforms as rekordbox draws them

### 4.0 Where the data comes from (important)

- **Detail waveform in normal use**: rekordbox does **not** draw the scrolling ("enlarged") waveform from the ANLZ zoom data. On load it decodes the audio and builds its own detail waveform with a bank of band-pass filters (`djplay::WaveZoom`, 18 `analyzer::BandPassFilter`s, `WaveDataZoom`) [BIN `UiPlayer::loadSongFile` 0x102215cf4, `changeWaveViewMode` 0x102214418] **(high)**. The ANLZ zoom data (`WaveCDJ`, from PWV3/PWV5/PWV7) is used only while the **MIX POINT LINK** panel is shown (setting `ShowPanel_MIX_POINT_LINK`). That path is the one that matches what CDJs show.
- **Overview**: always drawn from ANLZ, using `PWV4` (1200 points) for BLUE and RGB and `PWV6` for 3Band. `PWAV`/`PWV2` are not used for the desktop overview [BIN `WavPointCache::getWholeWavePoint` 0x101c7fa9c] **(high)**.
- **Implication for TuneOx**: rendering ANLZ data reproduces rekordbox's MIX-POINT-LINK / CDJ look exactly. To also match the smoother, higher-resolution detail waveform that rekordbox shows normally, the planned "own waveform generation" item will be needed.

### 4.1 Styles and defaults

- The preference is `ColorType`: **BLUE / RGB / 3Band** [M p.92, p.234]. Internally 0/2 = blue variants, 1 = RGB, 3 = 3Band. The code maps 0→PWV3, 1→PWV5, 2→PWV3, 3→PWV7 (table `{1,2,1,3}` at 0x10382b100) [BIN].
- **Default = 3 (3Band)** (`kWaveColorTypeDefaultValue` = 3) [BIN] **(high)**.
- **Overview form**: a Full/Preview setting picks **half** (bars grow up from the bottom) or **full** (mirrored around the centre). The default is `WholeWaveType` = 2; its mapping is not known. The manual screenshots show the half form [M p.232, BIN] **(medium)**.

### 4.2 BLUE

- **Detail (PWV3, 150 columns/s)**: each entry gives a 5-bit height and a 3-bit "whiteness" w. Colour = row w of an 8-entry table, **quantized to 3 bits per channel** before drawing. Whiteness ≥ 8 cannot occur, but is treated as row 1 [BIN `CAnalyzerIF::load_blueZoomWave` 0x10161ffe4, table `get_cdjZoomWaveRGBTable` at 0x1037e588c, expanded by `WaveCDJ::getColorItem`] **(high)**:

| w | table RGB | colour rekordbox actually draws (3-bit) |
| --- | --- | --- |
| 0 | #083C8C | **#002491** |
| 1 | #085DBD | **#0048B6** |
| 2 | #006DFF | **#006DFF** |
| 3 | #08AAF7 | **#00B6FF** |
| 4 | #52B6F7 | **#48B6FF** |
| 5 | #73AEFF | **#6DB6FF** |
| 6 | #73AEFF | **#6DB6FF** |
| 7 | #CEE7FF | **#DAFFFF** |

  Height: `h × 255 / 31` on a 0–255 scale.

- **Overview (PWV4 bytes 0–2)**: compute m = max(b0, b1) and i = clamp((m − b2) × 8 / m, 0, 7), with i = 7 when m = 0. Colour = row i of #106098, #1080B8, #0890F8, #10B8E8, #78C0E0, #90C0F8, #90C0F8, #D0E8F8 [BIN `WidgetLoudView::createWholeImage` 0x100fcbcc4, table built in `__GLOBAL__sub_I_djplay_WidgetLoudView.cpp`] **(high)**. Heights work as in RGB below.

### 4.3 RGB

- **Detail (PWV5)**: the big-endian u16 has bits 15–13 = **red**, 12–10 = **green**, 9–7 = **blue** and 6–2 = height, with bits 1–0 unused. This matches [DJL]. Colour = (r, g, b) × 255/7 and height = h × 255/31 [BIN `MstLoadColorZoomWave` 0x1007b69c4 + `load_colorZoomWave` + `WaveCDJ::getColorItem`] **(high)**.
  - beat-link's renderer swaps green and blue when building its colour; do not copy that.
  - In the fixtures, red tracks the low band and blue the high band (§4.4).
- **Overview (PWV4)**: colour = (b3, b4, b5) scaled so the largest channel is 255 (hue only). The main bar height comes from max(b3, b4, b5) and is drawn as a vertical gradient from the colour at 50 % alpha (far end) to the full colour (baseline). Underneath it, a white bar at 50 % alpha uses b5 for its height, which gives a lighter core [BIN `createWholeImage`] **(medium on the exact layering)**.

### 4.4 3Band

**Byte order (important)**: rekordbox reads `PWV6` and `PWV7` entries as **byte0 = low, byte1 = mid, byte2 = high**. Evidence:

- The overview path copies PWV6 bytes 0,1,2 verbatim into point bytes 3,4,5 [BIN `getWholeWavePoint`]. `create3BandsWholeImage` (0x100fcc38c) then draws byte3 as the blue bar, byte4 as amber on top and byte5 as white on top.
- The detail path (`draw3BandsWaveCDJ` 0x102190bf0) loads byte0 and byte1 into one vector, scales them by the first two coefficients and swaps the lanes. When it picks colours, the lane holding byte0 is the one drawn in **blue (low)**, the lane holding byte1 is drawn in **amber (mid)**, and byte2 goes through the cosine curve and is drawn **white (high)**. This was traced in the assembly, not just the decompiled C.
- In [ANLZ], PWV7 byte0 correlates with the RGB red field (r = 0.59–0.76) and byte2 with the blue field (r = 0.31–0.55).

[DJL] says "mid, high, low" and [BL] reads `[base+2]` as low. **Both disagree with rekordbox; go with rekordbox** **(high)**. The waveform agent should double-check with a test tone.

Fixed colours [BIN, in both 3-band paths]:

| Region | Colour |
| --- | --- |
| low only | **#0055E1** (blue) |
| mid only | **#FFA600** (amber) |
| high only | **#FFFFFF** |
| low ∩ mid | **#B4690A** (brown) |
| low ∩ high | **#D2DCFA** |
| mid ∩ high | **#FFF0D7** |
| all three | **#F5EBD7** (cream) |

**Height mapping**:

- Low and mid are linear in the byte, times a per-track coefficient.
- **High is compressed with a raised cosine**: h' = 0.5 − 0.5·cos(π·h/128) (an S-curve), then times its coefficient. The scale for 3-band data is 128 (`WaveDataCDJ::getResolution` = 128 for 3-band and 255 otherwise).
- **Coefficients**: per-track **`PWVC`** in `.2EX`. The 20-byte tag has `len_header` 0x0e and, after 2 zero bytes, three big-endian u16 coefficients (low, mid, high). rekordbox uses them **÷100**, e.g. 83/113/103 → 0.83/1.13/1.03, and defaults to 1.0 when PWVC is missing [BIN `read_pmai_ex2_wave_normalize_coef` 0x100786644, `DjplayDB::getWaveNormalizeCoef` 0x1001fece4, applied at `WaveView+0xb4..0xbc`] **(high)**.
  - This answers the spec's PWVC question: **rendering 3-band detail needs it**.
  - `PVDI` in the same file is vocal-detection data (`pmai_ex2_vocal_detect`) [BIN].

**Detail drawing (per pixel column, ANLZ path)**:

- Each band gives a vertical span: centred and mirrored for the full waveform, or anchored to the baseline for the half waveform.
- rekordbox draws the largest span in that band's pure colour, the middle span in the pairwise-overlap colour of the two larger bands, and the smallest span in the all-three cream.
- So the inner core is cream and the outer edges show the dominant band.
- When zoomed out to several entries per pixel, use each band's **maximum** over the column.

**Overview drawing (PWV6, 1200 points)**:

- Stacked bars from the bottom. Blue = 1.2·low. Amber is the next segment up, ending at 1.2·low + 0.6·mid. White is the top segment, ending at 1.2·low + 0.6·mid + 153.6·cosc(high), where cosc is the raised cosine above.
- All three are scaled by **M = the track's maximum of (low + mid + 128·cosc(high)), clamped to ≤ 128** and ≥ 1, and clipped to the view height.
- Draw order is white, then amber, then blue, so each covers the one before it.

**Smoothing, PCM path only**: rekordbox's own detail waveform applies peak-hold with release per band (low 0.99, mid 0.98, high 0.97 per sample) [BIN `create3BandsWaveInfo`]. Irrelevant to ANLZ rendering.

### 4.5 Common elements of the detail waveform

[BIN `WidgetWaveView::draw*GL`, colours from the `__GLOBAL__sub_I_djplay_WidgetWaveView2.cpp` initializer]:

- **Background**: black.
- **Playhead**: fixed at the horizontal centre. The waveform scrolls under it, and the playhead only moves off-centre near the track's start or end in some layouts.
  - Drawn as a 3 px black line at 50 % alpha (shadow) with a **1 px line on top**.
  - The 1 px line is **#E0E0E0 while playing** and **#FF0000 while paused/stopped**. The slip/background position is drawn in **#FFE000** [`drawCurrentCursorGL` 0x10211f0c0] **(medium on the exact state mapping)**.
- **Beat grid** [`drawBackgroundBeatGridGL` 0x10211e2e4] **(high on the colours, medium on the geometry)**:
  - **Downbeats** (beat 1 of each bar): a full-height 1 px **white** line with a black outline, plus **small red markers (#FF0000) at the top and bottom edges**.
  - **Other beats**: a 1 px white line at **30 % alpha** plus small white ticks at the top and bottom. These are drawn only when the visible span is under 120 s.
  - In GRID edit mode, changed grid lines turn blue (#20B8F8), with #F80000 for the first beat [M p.93, `drawForegroundBeatGridGL`].
- **Current cue**: an **orange up-triangle #F8A030 at the bottom** [RES `CurrentCueImageUp.svg`, M p.97].
- **Memory cues**: **down-triangles at the top**, in the memory/track colour (§5.2). With no colour assigned, rekordbox draws red #FF0000 (CDJ convention) [RES `memoryCueImageDown1.png`, BL] **(medium on the no-colour default)**.
- **Hot cues**: a label at the top (a coloured square with a dark letter A–P) and a thin vertical line in the hot cue colour [RES `hotCueImageAlpha.png` (one row per palette colour), `tXXXX_1pacue2.png`].
- **Loops**: translucent overlay (§2.7), with loop in/out cursors in #F80000/white.
- **Phrase bars** (when phrase analysis is on) run along the baseline. The bar counter is in accent blue.

### 4.6 Overview common elements

[BIN `WidgetLoudView::paintWave` 0x100fcdad4]:

- **Minute ticks**: every 60 s, 2 px tall at the bottom, light grey #C8C8C8.
- **Playhead**: a 1 px vertical line. It is the normal (light) colour while playing, **#FF0000 when paused**, **#FFE000 in slip**, and grey #707070 when the deck is inactive. An optional bold style is a 4 px black line with a 2 px white line inside.
- **Played portion**: rekordbox pre-renders two versions of the overview image and composites them **split at the playhead** (one image left of it, the other right of it). This is how the played part gets its distinct (dimmed) look. The exact dimming was not decoded **(medium)**.
- A horizontal "playing address" bar runs under the waveform [M p.69 item 8].
- **Markers**: hot cue letter squares above, memory cues on top, the current cue at the bottom-left, and active loops as coloured spans.

### 4.7 Zoom

- **PERFORMANCE**: zoom steps are visible spans of **1, 2, 3, 4, 8, 16, 24 s**, with a fallback default of the 5th step (8 s). MIX POINT LINK adds 8 s up to 30 min. Some vertical layouts use 0.25–6 s or 0.5–12 s tables [BIN `UiPlayer::getWaveRatioValueForDJ` 0x101acd7c8, tables at 0x1037f0bc8–0x1037f0c28] **(medium: span vs half-span)**.
- **EXPORT**: zoom is an integer N meaning N × 1000 ms (`getWaveRatioValueForRekordbox`), stored per player (`Player0_WaveViewRatio` = 12 in [USR]). The −/RST/+ buttons step it [M p.75]. The step size and default are not known.
- **Beat spacing on tempo change**: a preference decides what changes when the tempo changes. Either the beat spacing changes and scroll speed stays constant, or the scroll speed changes and beat spacing stays constant [BIN strings]. CDJs keep the beat spacing constant.

---

## 5. Cue colours

### 5.1 Hot cue palette

This is the palette behind **`djmdCue.ColorTableIndex`** and the ANLZ `PCO2` `color_code`. It is the same table as beat-link's `findRekordboxColor`, confirmed in the binary (`djplay::getPadColor`, table built in `__GLOBAL__sub_I_PadColour.cpp`) **(high)**. Index 0 = no colour, 63 = black, 64 = white. A separate saturated `getDevicePadColor` table drives hardware LEDs.

| idx | hex | idx | hex | idx | hex | idx | hex |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | #305AFF | 17 | #14AA7D | 33 | #E1AA00 | 49 | #DE44CF |
| 2 | #5073FF | 18 | #10B176 | 34 | #FFA000 | 50 | #DE448D |
| 3 | #508CFF | 19 | #30D26E | 35 | #FF9600 | 51 | #E630B4 |
| 4 | #50A0FF | 20 | #37DE5A | 36 | #FF8C00 | 52 | #E619DC |
| 5 | #50B4FF | 21 | #3CEB50 | 37 | #FF7500 | 53 | #E600FF |
| 6 | #50B0F2 | 22 | #28E214 | 38 | #E0641B | 54 | #DC00FF |
| 7 | #50AEE8 | 23 | #7DC13D | 39 | #E0461E | 55 | #CC00FF |
| 8 | #45ACDB | 24 | #8CC832 | 40 | #E0301E | 56 | #B432FF |
| 9 | #00E0FF | 25 | #9BD723 | 41 | #E02823 | 57 | #B93CFF |
| 10 | #19DAF0 | 26 | #A5E116 | 42 | #E62828 | 58 | #C542FF |
| 11 | #32D2E6 | 27 | #A5DC0A | 43 | #FF376F | 59 | #AA5AFF |
| 12 | #21B4B9 | 28 | #AAD208 | 44 | #FF2D6F | 60 | #AA72FF |
| 13 | #20AAA0 | 29 | #B4C805 | 45 | #FF127B | 61 | #8272FF |
| 14 | #1FA392 | 30 | #B4BE04 | 46 | #F51E8C | 62 | #6473FF |
| 15 | #19A08C | 31 | #BAB404 | 47 | #EB2DA0 | | |
| 16 | #14A584 | 32 | #C3AF04 | 48 | #E637B4 | | |

**Default colour when `ColorTableIndex` = 0**: comes from the **HOT CUE colour scheme** preference [M p.234]. Its default is **COLORFUL** (`getCurrentHotCueColorType` default 4). Per-slot defaults are in `rekordbox::hotCueColorIndex{CDJ,Cold1,Cold2,Colorfull}`: 17 bytes for slots A–P plus hot loops [BIN 0x10374d707…, `WidgetWaveView::getHotCueColor` 0x1021221a4] **(high)**.

| Scheme | A | B | C | D | E | F | G | H | I–P | hot loop |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| CDJ | 22 | 22 | 22 | 22 | 22 | 22 | 22 | 22 | 26 | 36 |
| COLD1 | 4 | 18 | 58 | 8 | 12 | 60 | 14 | 1 | = A–H | 36 |
| COLD2 | 14 | 11 | 8 | 5 | 3 | 1 | 61 | 59 | = A–H | 36 |
| **COLORFUL** | **43** #FF376F | **8** #45ACDB | **23** #7DC13D | **60** #AA72FF | **19** #30D26E | **38** #E0641B | **1** #305AFF | **32** #C3AF04 | = A–H | **36** #FF8C00 |

- An explicit `ColorTableIndex` > 0 wins over the scheme.
- The library in [DB] shows how this plays out:
  - Hot cue A, which is mostly rekordbox's automatic "1.1Bars" cue, has index 0 in 8,962 rows.
  - B–H mostly have **22 (green #28E214)**: 1,177 + 1,065 + 664 + 296 + 76 + 15 + 1 rows.
  - A few have 36, 42, 9 or 37.
  - `djmdCue.Color` is 255 on rows with a palette colour and −1 on rows without.

### 5.2 Memory cues and track colours

Memory cues use the **8 track colours**. `PCO2` `color_id` 1–8 = Pink, Red, Orange, Yellow, Green, Aqua, Blue, Purple; 0 = none [DJL]. rekordbox draws them as [RES `memoryCueImageDown1.png`, matching the `DeviceDefs` constants in BIN] **(high)**:

| Colour | Hex |
| --- | --- |
| Pink | #F870F8 |
| Red | #FF0000 |
| Orange | #F8A030 |
| Yellow | #F8E330 |
| Green | #00E000 |
| Aqua | #00C0F8 |
| Blue | #0050F8 |
| Purple | #9808F8 |
| none | white outline |

In [DB] every memory cue has `Color` −1 (434 rows) or 255 (8 rows) and `ColorTableIndex` 0, so the `djmdCue.Color` → 1–8 mapping for coloured memory cues is unconfirmed (open question).

### 5.3 UI accents

| Element | Colour |
| --- | --- |
| Accent blue (BEAT SYNC/MT/Q/EQ on) | #1473EB |
| CUE ring | #FFB937 |
| Panel greys | #282828 / #1E1E1E |
| PLAY ring | #646464 |

[RES SVGs]

---

## 6. Keyboard shortcuts (defaults)

**EXPORT mode** ("Export (Preset)") [USR `KeyMappings/rekordbox_0000000000030.mappings`]. The 7.0.5 PDF lists PERFORMANCE presets only and says EXPORT has its own, so these come from the key-map file rekordbox wrote **(medium-high)**. The keys below are for deck A; **add ⇧ for deck B**.

| Key | Action | Key | Action |
| --- | --- | --- | --- |
| Space | Play/Pause | C | Cue |
| ← / → | Jump reverse/forward (beat jump) | Q | Quantize |
| 1 / 2 / 3 | Hot cue A/B/C (set or call) | ⌘1/⌘2/⌘3 | Clear hot cue A/B/C |
| I / O / R | Loop in / out / exit-reloop | 4 5 6 7 8 9 | 1/2/4/8/16/32-beat loop |
| / , ⌥¥ | Loop ÷2, ×2 | M | Set memory cue |
| N / B | Call next/previous memory cue | X | Delete memory cue |
| A S D F G H J K L ; | Call memory cue 1–10 | T | Time mode (elapsed/remaining) |
| F1 | SYNC | F2 | Master tempo |
| F3 | Tempo reset | F4 / F5 | Pitch bend − / + |
| F6 / F7 | BPM − / + | F9 | Metronome sound |
| F10 / F11 / F12 | Show memory cues / hot cues / info | ⌘← / ⌘→ | Shift beat grid left/right |
| ⇧< / ⇧> | Shrink/expand beat interval | ⌘G | Adjust BPM/beat grid |
| ⌘↑ / ⌘↓ | Previous/next track | ⌘P / ⌘[ | Preview play/stop, skip |
| ⌘F12 / ⌘F11 / ⌘F10 | Volume up / down / mute | ⌘7 ⌘8 ⌘9 ⌘0 | Layouts |
| ⌘Z | Undo track load | ⌘L | Locate loaded track |

**PERFORMANCE "Performance 1 (Preset)"** [KB], deck 1 / deck 2:

| Action | Deck 1 | Deck 2 |
| --- | --- | --- |
| Play/Pause | Z | N |
| Cue | A | H |
| Back to track start | ⇧A | ⇧H |
| Jump | ⌘T / ⌘R | ⌘P / ⌘O |
| BEAT SYNC | Q | Y |
| Sync master | ⇧Q | ⇧Y |
| Pitch bend | E / W | I / U |
| Tempo | ⇧E / ⇧W | ⇧I / ⇧U |
| Call memory cue | T / R | P / O |
| Hot cues A–D | 1–4 | 6–9 |
| Loop in / out / exit | D / F / G | K / L / ; |
| Beat loops 1/2–8 | ⌘1–⌘5 | ⌘6–⌘0 |
| Load to deck | ⇧← | ⇧→ |

Zoom: `=` in, `-` out, ⇧`=` reset. "Performance 2" is a different layout [KB].

---

## Checklist: details that make it feel like rekordbox

### For the engine agent

1. **CUE**: follow §2.1 exactly.
   - CUE while paused sets the cue, snapped when Q is on.
   - CUE while playing jumps back to the cue and pauses.
   - Holding CUE plays while held, and returning to the cue on release.
   - **PLAY pressed while CUE is held keeps playing after release.**
2. **Hot cue on a saved slot jumps and plays, also when paused.** An empty slot stores the position, or the current loop as a hot loop. Gate mode is an optional preference, off by default.
3. **Memory cue call jumps and pauses.** Calling a memory loop and pressing PLAY loops.
4. **Auto beat loop**:
   - Starts at the current position, snapped to the beat with Q.
   - Pressing the same size again exits.
   - Halve/double are anchored at loop-in, over 1/64–512 beats.
   - Exit lets playback run past loop-out, and Reloop jumps back to loop-in.
   - Beat jump during a loop moves the loop.
5. **Beat jump sizes**: Fine, 4 beats, 8 beats, 16 beats, 8 bars, 16 bars, 32 bars. Default 4 beats.
6. **Quantize resolution 1 beat by default.** It snaps cue set, loop in/out, hot cue store, hot cue trigger and auto loop start. Quantize is **per deck** in rekordbox; a global toggle is acceptable if the UI shows it on both decks.
7. **Tempo**:
   - −/+ change by **0.02 %** per press, and **0.1 %** steps after holding 500 ms.
   - The BPM can be typed in.
   - RST resets to 0 %.
   - Allowed effective BPM is 40–499.
   - Keep ±6/10/16/WIDE for the slider model: 0.02/0.04/0.04/0.5 % resolution, WIDE = ±100 %.
8. **Pitch bend**: a temporary rate offset while the button is held, playing only.
9. **Sync**:
   - The first deck to start playing becomes master; MASTER hands it over.
   - When the master's track is unloaded or replaced, master passes to the other deck.
   - Slaves cannot change or reset tempo while synced.
   - Nudging a beat-synced slave drops it to **BPM-only sync** until SYNC is toggled; report this so the UI can show it.
   - Support the double/half BPM option; its default is an open question.
10. **Master tempo off by default.**
11. **Mixer** for the DUAL look: a crossfader, a per-band **kill** for low/mid/high, and a per-deck **Auto Gain**.
    - Auto Gain is linear gain from `djmdMixerParam` (float32 split over `GainHigh`/`GainLow`), shown in dB. User trims are saved with the track.
    - EQ knobs and channel faders are extras beyond the DUAL layout.
12. **Loading**:
    - EXPORT does not jump to the first memory cue; the deck parks at 0 with no cue.
    - Loading a track clears the cue and loop.
    - Starting a browser preview pauses the decks.

### For the UI agent

1. **DUAL layout**:
   - Deck A on top and deck B below.
   - **The enlarged waveforms are stacked in the middle as half-waveforms facing each other.** A grows up and B grows down, each with its phrase bar on its baseline.
   - One shared centre playhead.
   - Bar counter `N.bBars` in accent blue next to the playhead on each lane.
2. **Default style 3Band** with colours from §4.4:
   - Low #0055E1, mid #FFA600, high #FFFFFF.
   - Overlaps: #B4690A (low+mid), #D2DCFA (low+high), #FFF0D7 (mid+high), #F5EBD7 (all three).
   - The high band uses the raised-cosine curve.
   - Apply the per-track **PWVC** coefficients (÷100).
   - **PWV6/PWV7 byte order is low, mid, high.**
3. **BLUE and RGB** use the exact tables in §4.2–4.3. Detail colours are quantized to 3 bits per channel.
4. **Overview**:
   - 1200 points, half form.
   - Hot cue letter squares above it, memory cue down-triangles on top, and an orange current-cue triangle (#F8A030) at the bottom-left.
   - Minute ticks every 60 s in #C8C8C8.
   - Playhead light while playing, red when paused.
   - Played part drawn differently from the unplayed part.
   - Click to seek; playback state is kept.
5. **Detail waveform**:
   - Black background.
   - Playhead centred: 1 px #E0E0E0 over a 3 px 50 % black shadow while playing, red when paused.
   - Downbeats: full-height white lines with black outline plus red edge markers.
   - Other beats: 30 %-alpha white lines plus edge ticks, hidden when zoomed far out (span ≥ 120 s).
   - Hot cues: coloured letter flag and line.
   - Active loop: #FFA300 overlay at 50 %.
6. **Hot cue colours**:
   - The palette in §5.1.
   - When `ColorTableIndex` = 0, use the COLORFUL slot defaults: A #FF376F, B #45ACDB, C #7DC13D, D #AA72FF, E #30D26E, F #E0641B, G #305AFF, H #C3AF04.
   - Hot loops #FF8C00.
7. **Track info**:
   - BPM with two decimals.
   - Remaining time large with tenths (`-02:54.4`) and elapsed time grey.
   - Key with ± semitone shift.
   - KEY SYNC, BEAT SYNC (blue when on; **yellow/orange when it has fallen back to BPM sync**) and MASTER.
8. **Controls** per deck:
   - Operation column: `< >` jump, size dropdown, CUE (#FFB937 ring), PLAY.
   - DUAL CONTROL toggle between the decks.
   - Mixer column: A-GAIN knob with `+0.0dB` readout, HIGH/MID/LOW kill pills (#1473EB outline when on), vertical crossfader (A top, B bottom; double-click centres it).
   - Function row: MEMORY, AT/MT loop switch with `< N >`, `− BPM +` (double-click BPM to type), pitch bend ◄◄ ►►, MT, RST, Q.
   - Right side: a MEMORY / HOT CUE / INFO list per deck.
9. **Keyboard**: the EXPORT preset in §6, with plain keys for deck A and ⇧ for deck B. Space plays and pauses deck A.
10. **Waveform zoom**: −/RST/+ on the enlarged waveform. Optional: left-click play, right-click cue (off by default). Dragging the enlarged waveform scrubs.

---

## Open questions

1. **Byte order of PWV6/PWV7**: rekordbox's code reads low, mid, high, while DJL/beat-link say otherwise. The waveform agent should confirm with a synthetic bass-only or treble-only track analysed by rekordbox.
2. **Played-portion look of the overview**: two images are split at the playhead, but how the played one is dimmed (alpha or desaturation) was not decoded. A screenshot of rekordbox mid-track would settle it.
3. **Playhead colour** while paused away from the cue point. `doesNeedToBackCue` also selects the "normal" colour, so the exact states for red versus light need a live check.
4. **EXPORT zoom**: default N, step size, and whether N seconds is the full visible width or half of it.
5. **Pitch bend amount**: what `setJogRingSpeed(±0.1)` equals in percent.
6. **Crossfader curve** in EXPORT mode, and whether the Auto Gain knob really spans ±10 dB, given stored gains reach +14 dB.
7. **Defaults**: whether **Q** is on by default, whether the **double/half BPM sync** option is on by default, and the quantize "Mode" (immediate vs next beat) default. The getters default to 2 for `DeckQuantizeMode`/`DeckQuantizeStatus` and 1 for `BeatSyncType`; the value-to-meaning mapping was not decoded.
8. **Memory cue colours** in `djmdCue.Color` (1–8?) and the no-colour default (red vs white outline); no coloured memory cue exists in [DB].
9. What rekordbox writes to PCO2 `color_code`/RGB for hot cues whose `ColorTableIndex` is 0: the scheme default or 0 (= green on CDJs). This matters for the USB export.
10. **"Fine" beat jump** size in EXPORT; PERFORMANCE documents FINE = 5 ms.

---

## Spec changes needed (`docs/specs/decks.md`)

1. **B (waveform data)**:
   - State that `three_band` source bytes in PWV6/PWV7 are **low, mid, high** (rekordbox's reading; see §4.4).
   - Document PWVC: three u16 per-band normalisation coefficients ÷100, needed for rendering. `Waveform` (or `get_waveform`) should carry them, e.g. `band_gain: [f32; 3]`, so the UI can scale low/mid/high like rekordbox.
   - Add a "Later" note that rekordbox's normal detail waveform is generated from audio at load time; ANLZ gives the CDJ/MIX POINT LINK look.
2. **C (mixer)**: the DUAL PLAYER has per-band **kills** (on/off) and **Auto Gain**, not EQ knobs or channel faders.
   - Add `mixer_set` fields `eq_kill_low/mid/high` (bool), or define that `eq_*` = −∞ means kill and the UI toggles between 0 dB and −∞.
   - Add `auto_gain_db` per deck, loaded from `djmdMixerParam` and saved back when changed. Saving needs a library field and a write path.
3. **C (commands)**, needed for rekordbox parity:
   - `deck_tempo_step(deck, dir, fine|coarse)`, or document that the UI sends `deck_set_tempo` with ±0.02 %/±0.1 % steps.
   - `deck_set_bpm(deck, bpm)` for typed BPM.
   - `deck_tempo_reset`.
   - `deck_pitch_bend(deck, dir, press|release)`.
   - `deck_loop_in`, `deck_loop_out`, `deck_reloop`, `deck_loop_resize(deck, ×2|÷2)`.
   - `deck_memory_cue(deck, call next|prev|index, set, delete)`.
   - `deck_hot_cue_delete`.
   - `player_set_dual_control`.
   - Hot cue `press` on a stored slot plays, also when paused.
4. **C (DeckState)**:
   - Add `sync_mode: "off" | "beat" | "bpm"`, so the UI can show the yellow/orange fallback state.
   - Add `quantize` per deck if per-deck Q is adopted.
   - Add `slip_position_us: null` for later.
   - Add `last_loop` so RELOOP has a target.
5. **C (quantize)**: rekordbox has Q **per deck**. Either make `player_set_quantize` per deck (`deck_set_quantize`) or keep it global and note the deviation. Quantize resolution is 1 beat.
6. **C (tempo)**: keep ±6/10/16/WIDE, document the resolutions (0.02/0.04/0.04/0.5 %), and add the 40–499 BPM clamp. Master tempo stays out of scope; rekordbox defaults it to off.
7. **D (UI)**:
   - Add the stacked, mirrored half-waveforms for the DUAL layout and the bar counter.
   - Add the remaining-time display with tenths.
   - Add KEY SYNC and key shift, which can be display-only until key lock exists.
   - Add the MEMORY / HOT CUE side lists and the Matching button (optional).
   - Add the default **3Band** style.
   - Add the EXPORT keyboard map (§6).
   - "Played part dimmed" stays, pending open question 2.
