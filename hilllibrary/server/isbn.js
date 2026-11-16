// ISBN validation and conversion. Checksum rules per ISO 2108 (ISBN-10: weighted
// mod 11, ISBN-13: alternating 1/3 weights mod 10).

export function normalize(input) {
  return String(input ?? '').replace(/[\s-]/g, '').toUpperCase();
}

function check13(first12) {
  let sum = 0;
  for (let i = 0; i < 12; i++) sum += Number(first12[i]) * (i % 2 ? 3 : 1);
  return String((10 - (sum % 10)) % 10);
}

export function isValidIsbn10(s) {
  if (!/^\d{9}[\dX]$/.test(s)) return false;
  let sum = 0;
  for (let i = 0; i < 10; i++) sum += (s[i] === 'X' ? 10 : Number(s[i])) * (10 - i);
  return sum % 11 === 0;
}

export function isValidIsbn13(s) {
  return /^97[89]\d{10}$/.test(s) && check13(s) === s[12];
}

/** Returns a canonical ISBN-13 string, or null if the input is not a valid ISBN-10/13. */
export function toIsbn13(input) {
  const s = normalize(input);
  if (isValidIsbn13(s)) return s;
  if (isValidIsbn10(s)) {
    const core = '978' + s.slice(0, 9);
    return core + check13(core);
  }
  return null;
}

/** ISBN-10 equivalent of a 978-prefixed ISBN-13, else null (979 has no ISBN-10). */
export function toIsbn10(isbn13) {
  if (!isValidIsbn13(isbn13) || !isbn13.startsWith('978')) return null;
  const core = isbn13.slice(3, 12);
  let sum = 0;
  for (let i = 0; i < 9; i++) sum += Number(core[i]) * (10 - i);
  const c = (11 - (sum % 11)) % 11;
  return core + (c === 10 ? 'X' : String(c));
}
