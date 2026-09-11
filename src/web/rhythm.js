'use strict';
// Pure view calculations. Each minute retains every state and unrecorded gap.
const FlowRhythm = (() => {
  const key = segment => `${segment.sample.id}:${segment.start}`;
  function hourRows(date, segments) {
    if (!segments.length) return [];
    const dayStart = new Date(`${date}T00:00:00`).getTime();
    const nextDay = new Date(dayStart); nextDay.setDate(nextDay.getDate() + 1);
    const dayEnd = nextDay.getTime();
    const ordered = [...segments].sort((a, b) => a.start - b.start);
    const start = Math.max(dayStart, dayStart + Math.floor((ordered[0].start - dayStart) / 3600000) * 3600000);
    const end = Math.min(dayEnd, dayStart + Math.ceil((ordered.at(-1).end - dayStart) / 3600000) * 3600000);
    const rows = []; let index = 0;
    for (let hour = start; hour < end; hour += 3600000) {
      const cells = [];
      for (let minute = hour; minute < Math.min(hour + 3600000, dayEnd); minute += 60000) {
        const stop = minute + 60000, parts = []; let cursor = minute;
        while (index < ordered.length && ordered[index].end <= minute) index++;
        for (let i = index; i < ordered.length && ordered[i].start < stop; i++) {
          const s = ordered[i], a = Math.max(cursor, s.start), b = Math.min(stop, s.end);
          if (b <= a) continue;
          if (a > cursor) parts.push({category: 'unobserved', start: cursor, end: a});
          parts.push({category: s.category, start: a, end: b, key: key(s)});
          cursor = b;
        }
        if (cursor < stop) parts.push({category: 'unobserved', start: cursor, end: stop});
        cells.push({start: minute, end: stop, parts});
      }
      rows.push({start: hour, cells});
    }
    return rows;
  }
  function hit(cell, ratio) {
    const at = cell.start + Math.max(0, Math.min(.999999, ratio)) * (cell.end - cell.start);
    return cell.parts.find(p => p.start <= at && p.end > at);
  }
  return {hourRows, hit};
})();
if (typeof module !== 'undefined') module.exports = FlowRhythm;
