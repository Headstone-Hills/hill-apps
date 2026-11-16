// Small text helpers shared by the database and metadata layers.

const PARTICLES = new Set(['le', 'la', 'de', 'del', 'della', 'di', 'da', 'van', 'von', 'der', 'den', 'du', 'el', 'al', 'bin', 'ibn', 'st', 'st.', 'mac']);
const SUFFIXES = new Set(['jr', 'jr.', 'sr', 'sr.', 'ii', 'iii', 'iv', 'phd', 'md']);

/** "Ursula K. Le Guin" -> "le guin ursula k"; "Martin Luther King Jr." -> "king martin luther". */
export function authorSortKey(name) {
  let parts = String(name ?? '').replace(/[,]/g, ' ').trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return '';
  while (parts.length > 1 && SUFFIXES.has(parts[parts.length - 1].toLowerCase())) parts.pop();
  if (parts.length === 1) return parts[0].toLowerCase();
  let i = parts.length - 1; // index where the surname starts
  while (i > 1 && PARTICLES.has(parts[i - 1].toLowerCase())) i--;
  return [...parts.slice(i), ...parts.slice(0, i)].join(' ').toLowerCase();
}

/** Ignore leading articles when alphabetising: "The Hobbit" -> "hobbit". */
export function titleSortKey(title) {
  return String(title ?? '').trim().toLowerCase().replace(/^(the|a|an)\s+/, '');
}

export function stripHtml(s) {
  return String(s ?? '')
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/p>/gi, '\n')
    .replace(/<[^>]+>/g, '')
    .replace(/&amp;/g, '&').replace(/&quot;/g, '"').replace(/&#39;|&apos;/g, "'")
    .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&nbsp;/g, ' ')
    .replace(/\n{3,}/g, '\n\n').trim();
}
