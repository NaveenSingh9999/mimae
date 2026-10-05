// Conformance test for the mimae-node binding: a full coordination loop
// driven from JS — team lifecycle, planning, assignment, status flow,
// progress, event polling, and error paths. Run with `npm test` after
// `cargo build -p mimae-node` (test.js loads the built .node file).
const assert = require('assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const mimae = require('./mimae_node.node');

const B = (n) => BigInt(n);
const J = (v) => JSON.stringify(v);

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'mimae-node-test-'));
const logPath = path.join(tmp, 'run.log');

// Wire version matches the core protocol.
assert.strictEqual(mimae.protocolVersion(), 1);

const h = mimae.open(logPath);
assert.strictEqual(typeof h, 'number');

const coord = J({ kind: 'coordinator' });
const pub = (payload, at) =>
  JSON.parse(mimae.publish(h, coord, J(payload), '[]', B(at)));

// Team lifecycle.
pub({ type: 'team_created', team: { id: 1, head: 1, mode: 'auto', state: 'initializing', members: [1, 2] } }, 1);
for (const id of [1, 2]) {
  pub({ type: 'agent_registered', agent: { id, cli: 'open_code', capabilities: ['backend'], role: null, state: 'idle', budget_used_tokens: 0 } }, 2);
  pub({ type: 'agent_ready', agent_id: id }, 3);
}

// Plan a two-task goal.
const goal = { id: 10, text: 'ship it', acceptance: ['done'], constraints: [], budget: { max_tokens: 0, deadline_ms: null } };
const task = (id, scope) => ({
  id, title: `t${id}`, brief: `brief ${id}`, capability: 'backend',
  write_scope: { paths: [scope] }, deps: [], verification: 'none', interfaces: [],
});
const receipt = JSON.parse(mimae.planGoal(h, J(goal), J([task(1, 'src/a'), task(2, 'src/b')]), '[]', B(12)));
assert.strictEqual(receipt.task_seqs.length, 2);

// Ready both tasks, assign agent 1, walk task 1 to Done.
for (const id of [1, 2]) {
  pub({ type: 'task_status_changed', task_id: id, from: 'proposed', to: 'ready', reason: null }, 20);
}
const assigned = JSON.parse(mimae.assign(h, B(1), B(30), B(60000)));
assert.strictEqual(assigned.task_id, 1);
assert.ok(assigned.lease, 'assign returns a lease');
for (const [from, to] of [['claimed', 'in_progress'], ['in_progress', 'in_review'], ['in_review', 'integrating'], ['integrating', 'done']]) {
  pub({ type: 'task_status_changed', task_id: 1, from, to, reason: null }, 40);
}
let progress = JSON.parse(mimae.goalProgress(h, B(10)));
assert.deepStrictEqual([progress.total, progress.done, progress.failed], [2, 1, 0]);

// Agent 2 takes task 2 to Done; nothing is left to sweep.
const assigned2 = JSON.parse(mimae.assign(h, B(2), B(50), B(60000)));
assert.strictEqual(assigned2.task_id, 2);
for (const [from, to] of [['claimed', 'in_progress'], ['in_progress', 'in_review'], ['in_review', 'integrating'], ['integrating', 'done']]) {
  pub({ type: 'task_status_changed', task_id: 2, from, to, reason: null }, 60);
}
progress = JSON.parse(mimae.goalProgress(h, B(10)));
assert.deepStrictEqual([progress.total, progress.done, progress.failed], [2, 2, 0]);
assert.deepStrictEqual(JSON.parse(mimae.sweepExpired(h, B(70))), []);

// Event subscription sees the whole run.
const events = JSON.parse(mimae.eventsSince(h, B(0)));
assert.ok(events.length > 15, `expected a full log, saw ${events.length} events`);
assert.strictEqual(JSON.parse(mimae.eventsSince(h, B(events.length))).length, 0);

// Error paths throw with readable messages. Unknown payload types are
// preserved by design (forward compat), but malformed known payloads fail.
const unknown = pub({ type: 'future_kind', anything: 1 }, 80);
assert.strictEqual(unknown.payload.type, 'future_kind');
assert.throws(() => mimae.publish(h, coord, J({ type: 'task_status_changed', task_id: 1 }), '[]', B(81)), /./);
assert.throws(() => mimae.claim(h, B(999), B(1), J({ paths: [] }), B(80), B(60000)), /./);
assert.throws(() => mimae.close(424242), /unknown coordinator handle/);

mimae.close(h);
console.log(`mimae-node conformance passed (${events.length} events, goal 2/2 done)`);
