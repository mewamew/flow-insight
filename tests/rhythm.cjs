'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const {hourRows, hit} = require('../src/web/rhythm.js');
const at = clock => new Date(`2026-09-09T${clock}`).getTime();
const segment = (id, start, end, category) => ({sample:{id},start:at(start),end:at(end),category});

test('one minute retains mixed states and unrecorded gaps, with exact selection', () => {
  const segments = [segment('a','09:00:05','09:00:20','work'),segment('b','09:00:30','09:00:55','distracted')];
  const rows = hourRows('2026-09-09', segments), cell = rows[0].cells[0];
  assert.equal(rows.length, 1);assert.equal(rows[0].cells.length, 60);
  assert.deepEqual(cell.parts.map(p=>[p.category,(p.end-p.start)/1000]), [['unobserved',5],['work',15],['unobserved',10],['distracted',25],['unobserved',5]]);
  assert.equal(hit(cell,.25).key, `a:${at('09:00:05')}`);
  assert.equal(hit(cell,.4).key, undefined);
  assert.equal(hit(cell,.5).key, `b:${at('09:00:30')}`);
  assert.equal(hit(cell,1).category, 'unobserved');
  assert.equal(rows[0].cells[1].parts[0].category, 'unobserved');
});

test('cross-hour segments retain identity, unknown and away remain distinct', () => {
  const rows=hourRows('2026-09-09',[segment('a','09:59:40','10:00:20','unknown'),segment('b','10:00:20','10:01:00','away')]);
  assert.equal(rows.length,2);
  assert.equal(hit(rows[0].cells[59],.9).key,hit(rows[1].cells[0],.1).key);
  assert.equal(hit(rows[1].cells[0],.1).category,'unknown');
  assert.equal(hit(rows[1].cells[0],.8).category,'away');
});

test('empty days remain empty and midnight clipping does not create extra rows', () => {
  assert.deepEqual(hourRows('2026-09-09',[]),[]);
  const start=at('23:59:30'),end=at('23:59:59')+31_000;
  const rows=hourRows('2026-09-09',[{sample:{id:'night'},start,end,category:'work'}]);
  assert.equal(rows.length,1);
  assert.equal(rows[0].cells.at(-1).end,at('00:00:00')+86400000);
  assert.equal(rows[0].cells.at(-1).parts.at(-1).end,at('00:00:00')+86400000);
});

test('hours without observations remain visibly unrecorded', () => {
  const rows=hourRows('2026-09-09',[segment('a','09:00:00','09:01:00','work'),segment('b','11:00:00','11:01:00','work')]);
  assert.equal(rows.length,3);
  assert.ok(rows[1].cells.every(c=>c.parts.length===1&&c.parts[0].category==='unobserved'));
});
