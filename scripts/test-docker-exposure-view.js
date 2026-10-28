#!/usr/bin/env node
'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

// LuCI appends non-node array items as text, including null. Keep this
// behavior in the regression fixture instead of silently dropping empty items.
function E(tag, attrs, data) {
    const children = Array.isArray(data)
        ? data.map(child => child && child.nodeType ? child : String(child))
        : data == null ? [] : [data && data.nodeType ? data : String(data)];
    return { nodeType: 1, tag, attrs: attrs || {}, children };
}

const source = fs.readFileSync(path.join(__dirname, '../packages/luci-app-docker-exposure/htdocs/luci-static/resources/view/docker-exposure/overview.js'), 'utf8');
const page = new Function('view', 'rpc', 'ui', 'poll', 'E', source)(
    { extend: value => value }, { declare: () => () => Promise.resolve({}) }, {}, {}, E
);
page.pending = {};
page.busy = false;
page.paint = () => {};
const container = {
    name: 'example-container', network: 'bridge', mode: 'lan', state: 'guarded',
    configured: true, ports: ['12345/tcp'], total_ports: 1, covered_ports: 1,
    bindings: [{ host_ip: '0.0.0.0', host_port: '12345', container_port: '80', protocol: 'tcp' }]
};
const accessCell = () => page.row(container).children[2];

assert.equal(accessCell().children.length, 1, 'unchanged row must not contain a placeholder text node');
assert.equal(accessCell().children[0].attrs.class, 'de-segment');
page.choose(container, 'wan');
assert.equal(accessCell().children.length, 2);
assert.equal(accessCell().children[1].attrs.class, 'de-unsaved');
page.choose(container, 'lan');
assert.equal(accessCell().children.length, 1, 'undo must remove the pending message without adding null');
container.configured = false;
container.state = 'unconfigured';
assert.equal(accessCell().children.length, 1);
assert.equal(accessCell().children[0].tag, 'span');
console.log('PASS LuCI-compatible row rendering before edit, pending edit, undo and unconfigured state');
