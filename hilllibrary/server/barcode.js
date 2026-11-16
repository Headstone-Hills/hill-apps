// Retail barcode (GTIN) validation for DVDs and Blu-rays. Discs carry a UPC-A (12 digits)
// or EAN-13 barcode; both use the same weighted mod-10 check digit (GS1 General Specifications),
// and a UPC-A is just an EAN-13 with a leading zero, so everything is normalised to 13 digits.

/** Canonical 13-digit GTIN for a valid UPC-A / EAN-13, else null. */
export function toGtin13(input) {
  let s = String(input ?? '').replace(/[\s-]/g, '');
  if (!/^\d{12,13}$/.test(s)) return null;
  if (s.length === 12) s = '0' + s;
  let sum = 0;
  for (let i = 0; i < 12; i++) sum += Number(s[i]) * (i % 2 ? 3 : 1);
  return (10 - (sum % 10)) % 10 === Number(s[12]) ? s : null;
}

export const DISC_MEDIA = ['dvd', 'bluray'];
export const isDisc = (m) => DISC_MEDIA.includes(m);
