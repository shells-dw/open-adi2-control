'use strict';
/**
 * EQ response for the curve display: RBJ "Audio EQ Cookbook" biquads, evaluated at a high
 * sample rate so the curve looks like the analog prototype up to 20 kHz.
 *
 * EQ indices (protocol): band n gain/freq/Q, band 1 type 3, band 5 type 16,
 * Bass 21/22/23, Treble 24/25/26. Gains in 0.5 dB steps, Q in 0.1 steps, frequencies in Hz.
 */
const EQ = (() => {
  const FS = 192000;
  const BANDS = [
    { band: 1, g: 4, f: 5, q: 6, type: 3 },
    { band: 2, g: 7, f: 8, q: 9 },
    { band: 3, g: 10, f: 11, q: 12 },
    { band: 4, g: 13, f: 14, q: 15 },
    { band: 5, g: 17, f: 18, q: 19, type: 16 },
  ];
  const BASS = { band: 'B', g: 21, f: 22, q: 23 };
  const TREBLE = { band: 'T', g: 24, f: 25, q: 26 };

  function coeffs(kind, f, gainDb, q) {
    const A = Math.pow(10, gainDb / 40);
    const w0 = (2 * Math.PI * Math.min(f, FS * 0.45)) / FS;
    const cs = Math.cos(w0), sn = Math.sin(w0);
    const alpha = sn / (2 * Math.max(q, 0.1));
    const sA = 2 * Math.sqrt(A) * alpha;
    switch (kind) {
      case 'peak': return [1 + alpha * A, -2 * cs, 1 - alpha * A, 1 + alpha / A, -2 * cs, 1 - alpha / A];
      case 'lowshelf': return [A * ((A + 1) - (A - 1) * cs + sA), 2 * A * ((A - 1) - (A + 1) * cs), A * ((A + 1) - (A - 1) * cs - sA),
        (A + 1) + (A - 1) * cs + sA, -2 * ((A - 1) + (A + 1) * cs), (A + 1) + (A - 1) * cs - sA];
      case 'highshelf': return [A * ((A + 1) + (A - 1) * cs + sA), -2 * A * ((A - 1) + (A + 1) * cs), A * ((A + 1) + (A - 1) * cs - sA),
        (A + 1) - (A - 1) * cs + sA, 2 * ((A - 1) - (A + 1) * cs), (A + 1) - (A - 1) * cs - sA];
      case 'lowpass': return [(1 - cs) / 2, 1 - cs, (1 - cs) / 2, 1 + alpha, -2 * cs, 1 - alpha];
      case 'highpass': return [(1 + cs) / 2, -(1 + cs), (1 + cs) / 2, 1 + alpha, -2 * cs, 1 - alpha];
      default: return [1, 0, 0, 1, 0, 0];
    }
  }

  function magDb(c, f) {
    const w = (2 * Math.PI * f) / FS;
    const c1 = Math.cos(w), s1 = Math.sin(w), c2 = Math.cos(2 * w), s2 = Math.sin(2 * w);
    const nr = c[0] + c[1] * c1 + c[2] * c2, ni = -(c[1] * s1 + c[2] * s2);
    const dr = c[3] + c[4] * c1 + c[5] * c2, di = -(c[4] * s1 + c[5] * s2);
    return 10 * Math.log10((nr * nr + ni * ni) / (dr * dr + di * di));
  }

  /** Filter kind of a band from its type value. */
  function kindOf(b, typeVal) {
    if (b.band === 'B') return 'lowshelf';
    if (b.band === 'T') return 'highshelf';
    if (b.band === 1) return ['peak', 'lowshelf', 'highpass', 'lowpass'][typeVal] || 'peak';
    if (b.band === 5) return ['peak', 'highshelf', 'lowpass'][typeVal] || 'peak';
    return 'peak';
  }

  /**
   * Filters of one EQ side. get(idx) returns the protocol integer.
   * withBT: include Bass/Treble.
   */
  function filters(get, withBT) {
    const out = [];
    const list = withBT ? [...BANDS, BASS, TREBLE] : BANDS;
    for (const b of list) {
      const g = (get(b.g) ?? 0) * 0.5, f = get(b.f) ?? 1000, q = (get(b.q) ?? 10) / 10;
      const kind = kindOf(b, b.type ? get(b.type) ?? 0 : 0);
      if ((kind === 'peak' || kind.endsWith('shelf')) && g === 0) continue;
      out.push(coeffs(kind, f, g, q));
    }
    return out;
  }

  /** Response in dB at frequency f. */
  function response(fl, f) {
    let db = 0;
    for (const c of fl) db += magDb(c, f);
    return db;
  }

  return { BANDS, BASS, TREBLE, filters, response, kindOf };
})();
