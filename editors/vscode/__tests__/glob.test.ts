// The tests of matching paths against zenith's globs, as zenith_utils matches
// them.

import * as assert from "node:assert/strict";

import { globMatches } from "../src/glob";

describe("globMatches", () => {
  it("matches a segment with `*` and `?`, and literal paths", () => {
    assert.ok(globMatches("apps/*", "apps/web"));
    assert.ok(!globMatches("apps/*", "apps/web/src"));
    assert.ok(!globMatches("apps/*", "apps"));
    assert.ok(globMatches("apps/we?", "apps/web"));
    assert.ok(globMatches("./apps/web", "apps/web"));
    assert.ok(globMatches("app/(dashboard)/[id]", "app/(dashboard)/[id]"));
    assert.ok(!globMatches("apps/*", ""));
  });

  it("matches any number of segments with `**`, and one or more at the end", () => {
    assert.ok(globMatches("services/**", "services/core"));
    assert.ok(globMatches("services/**", "services/core/api"));
    assert.ok(!globMatches("services/**", "services"));
    assert.ok(globMatches("**/api", "api"));
    assert.ok(globMatches("**/api", "services/core/api"));
    assert.ok(globMatches("a/**/b", "a/b"));
    assert.ok(globMatches("a/**/b", "a/x/y/b"));
    assert.ok(globMatches("**", "a/b"));
  });

  it("expands braces, which may nest, and escapes", () => {
    assert.ok(globMatches("{apps,libs}/*", "libs/ui"));
    assert.ok(globMatches("apps/{web,{admin,docs}}", "apps/docs"));
    assert.ok(!globMatches("apps/{web,admin}", "apps/docs"));
    assert.ok(globMatches("apps/\\*", "apps/*"));
    assert.ok(!globMatches("apps/\\*", "apps/web"));
    assert.ok(!globMatches("apps/{web", "apps/web"));
  });
});
