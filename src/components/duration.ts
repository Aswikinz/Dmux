/** Plain-language durations for estimates ("about 1–2 minutes"). */

function unit(seconds: number): { value: number; name: string } {
  if (seconds < 90) return { value: Math.max(1, Math.round(seconds)), name: 'second' };
  if (seconds < 90 * 60) return { value: Math.round(seconds / 60), name: 'minute' };
  return { value: Math.round((seconds / 3600) * 10) / 10, name: 'hour' };
}

function plural(value: number, name: string): string {
  return `${value} ${name}${value === 1 ? '' : 's'}`;
}

/** One duration in words, e.g. "about 3 minutes". */
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return 'under a second';
  const u = unit(seconds);
  return `about ${plural(u.value, u.name)}`;
}

/** A range in words, e.g. "about 1–2 minutes" or "about 40 seconds to 2 minutes". */
export function formatDurationRange(min: number, max: number): string {
  if (!Number.isFinite(min) || !Number.isFinite(max) || max <= 0) {
    return 'not known yet';
  }
  const loSec = Math.max(0, Math.min(min, max));
  const hi = unit(Math.max(min, max));
  const size = hi.name === 'hour' ? 3600 : hi.name === 'minute' ? 60 : 1;
  const loInHi = hi.name === 'hour' ? Math.round((loSec / size) * 10) / 10 : Math.round(loSec / size);
  if (loInHi >= 1 || hi.name === 'second') {
    const lo = Math.max(1, loInHi);
    if (lo === hi.value) return `about ${plural(hi.value, hi.name)}`;
    return `about ${lo}–${plural(hi.value, hi.name)}`;
  }
  const lo = unit(loSec);
  return `about ${plural(lo.value, lo.name)} to ${plural(hi.value, hi.name)}`;
}
