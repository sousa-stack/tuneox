import { useEffect, useState } from "react";
import { api, type Cue, type TrackDetails } from "../api";
import { camelot, formatDuration, formatPosition } from "../model";

export function TrackDetail({ trackId }: { trackId: number | null }) {
  const [details, setDetails] = useState<TrackDetails | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (trackId == null) {
      setDetails(null);
      return;
    }
    let current = true;
    api
      .track(trackId)
      .then((d) => current && (setDetails(d), setError(null)))
      .catch((e) => current && setError(String(e)));
    return () => {
      current = false;
    };
  }, [trackId]);

  if (trackId == null) {
    return <aside className="detail detail-empty">Select a track to see its cues and beat grid.</aside>;
  }
  if (error) return <aside className="detail detail-empty">{error}</aside>;
  if (!details || details.track.id !== trackId) return <aside className="detail" />;

  const { track, cues, beat_grid } = details;
  const facts: [string, string | number | null][] = [
    ["BPM", track.bpm?.toFixed(2) ?? null],
    ["Key", track.key ? `${camelot(track.key)} · ${track.key}` : null],
    ["Time", formatDuration(track.duration_secs) || null],
    ["Genre", track.genre],
    ["Label", track.label],
    ["Year", track.year],
    ["Plays", track.play_count || null],
    [
      "Format",
      [track.file_type?.toUpperCase(), track.bit_depth && `${track.bit_depth}-bit`, track.sample_rate && `${(track.sample_rate / 1000).toFixed(1)} kHz`]
        .filter(Boolean)
        .join(" · ") || null,
    ],
  ];

  return (
    <aside className="detail">
      <div className="detail-title">{track.title}</div>
      <div className="detail-artist">{track.artist ?? "Unknown artist"}</div>
      {track.album && <div className="detail-album">{track.album}</div>}

      <dl className="facts">
        {facts
          .filter(([, value]) => value != null && value !== "")
          .map(([label, value]) => (
            <div key={label} className={label === "Format" ? "wide" : undefined}>
              <dt>{label}</dt>
              <dd>{value}</dd>
            </div>
          ))}
      </dl>

      <section>
        <h3>Beat grid</h3>
        {beat_grid ? (
          <p className="grid-summary">
            {beat_grid.beats.toLocaleString()} beats ·{" "}
            {beat_grid.constant_bpm != null ? `${beat_grid.constant_bpm.toFixed(2)} BPM` : "variable tempo"} · first beat at{" "}
            {formatPosition(beat_grid.first_beat_us)}
          </p>
        ) : (
          <p className="muted">Not analyzed yet.</p>
        )}
      </section>

      <section>
        <h3>Cues {cues.length > 0 && <span className="muted">{cues.length}</span>}</h3>
        {cues.length === 0 ? <p className="muted">No cues.</p> : <ul className="cues">{cues.map(cueRow)}</ul>}
      </section>

      <section>
        <h3>File</h3>
        <p className="path" title={track.path}>
          {track.path}
        </p>
      </section>
    </aside>
  );
}

function cueRow(cue: Cue, index: number) {
  const hot = cue.hot_cue != null;
  const loop = cue.end_us != null;
  const label = hot ? String.fromCharCode(65 + cue.hot_cue!) : loop ? "LOOP" : "MEM";
  return (
    <li key={index} className="cue">
      <span className={`cue-badge ${hot ? "hot" : "memory"}${loop ? " loop" : ""}`}>{label}</span>
      <span className="mono">{formatPosition(cue.start_us)}</span>
      {loop && <span className="muted mono">{((cue.end_us! - cue.start_us) / 1e6).toFixed(2)} s loop</span>}
      {cue.comment && <span className="cue-comment">{cue.comment}</span>}
    </li>
  );
}
