/// <reference types="node" />
// Runs the shared routing fixtures (fixtures/routing.json) against routing.ts.
import { readFileSync } from "node:fs";
import { test } from "node:test";
import assert from "node:assert/strict";

import { route, type AnchorSpec, type Rect, type RoutingStyle } from "./routing.ts";

interface Case {
  name: string;
  from: Rect;
  to: Rect;
  from_anchor: AnchorSpec;
  to_anchor: AnchorSpec;
  routing: RoutingStyle;
  expect: unknown;
}

const fixtures = JSON.parse(
  readFileSync(new URL("../../../fixtures/routing.json", import.meta.url), "utf8"),
) as { cases: Case[] };

for (const c of fixtures.cases) {
  test(c.name, () => {
    const r = route(c.from, c.to, c.from_anchor, c.to_anchor, c.routing);
    assert.deepEqual(JSON.parse(JSON.stringify(r)), c.expect);
  });
}
