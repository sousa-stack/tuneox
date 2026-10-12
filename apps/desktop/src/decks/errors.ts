// Command errors as the window shows them. The engine and MCP API number the
// decks 1 and 2; the UI labels them A and B, so toasts translate, and turn
// the API's "x: y" / "x; y" clauses into sentences.

const LETTER: Record<string, string> = { "1": "A", "2": "B" };

/** "deck 2" → "Deck B" wherever the API names a deck. */
function deckLetters(text: string): string {
  return text.replace(/\bdeck ([12])\b/gi, (_, n: string) => `Deck ${LETTER[n]}`);
}

function sentence(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) return trimmed;
  const capital = trimmed[0].toUpperCase() + trimmed.slice(1);
  return /[.!?…]$/.test(capital) ? capital : `${capital}.`;
}

/** A command error message, worded for the deck UI. */
export function friendlyError(raw: string): string {
  let message = raw.replace(/^Error:\s*/, "").replace(/^invalid input:\s*/, "");

  // Tempo changes on a synced deck (tuneox_audio::Error::FollowsMaster).
  const synced = /^deck ([12]) is synced to the master, deck ([12]): change the tempo on deck \2 or turn sync off on deck \1$/.exec(message);
  if (synced) {
    const [, slave, master] = synced;
    return `Deck ${LETTER[slave]} is synced to Deck ${LETTER[master]}. Change the tempo on Deck ${LETTER[master]}, or turn off BEAT SYNC on Deck ${LETTER[slave]}.`;
  }
  // "there are two decks, 1 and 2; there's no deck 3" names numbers on purpose.
  if (/two decks, 1 and 2/.test(message)) return sentence(message.replace("two decks, 1 and 2", "two decks, A and B"));

  if (!/\bdeck [12]\b/i.test(message)) return message;
  message = deckLetters(message);
  // Clauses the API joins with ": " or "; " become sentences.
  const parts = message.split(/(?::|;) (?=[a-z])/);
  return parts.map(sentence).join(" ");
}
