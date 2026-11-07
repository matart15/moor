import { describe, expect, it } from "vite-plus/test";
import {
  argsToArrayOrNull,
  argsToArrayOrUndefined,
  entriesToRecordOrNull,
  entriesToRecordOrUndefined,
  findDuplicateHeaderKeys,
  findDuplicateKeys,
  formToUpdates,
  headerEntriesToRecordOrNull,
  headerEntriesToRecordOrUndefined,
} from "./server-form";
import type { ServerUpdateInput } from "@moor/types";

describe("server form utilities", () => {
  it("uses undefined for empty create fields and null for empty update fields", () => {
    expect(entriesToRecordOrUndefined([["", "ignored"]])).toBeUndefined();
    expect(entriesToRecordOrNull([["", "ignored"]])).toBeNull();
    expect(argsToArrayOrUndefined(" \n ")).toBeUndefined();
    expect(argsToArrayOrNull(" \n ")).toBeNull();
  });

  it("trims keys and argument lines while preserving values", () => {
    expect(
      entriesToRecordOrUndefined([
        [" TOKEN ", " secret "],
        ["EMPTY", ""],
      ]),
    ).toEqual({ TOKEN: " secret ", EMPTY: "" });
    expect(argsToArrayOrUndefined(" one \n\n two ")).toEqual(["one", "two"]);
  });

  it("parses pasted argument lines with Windows line endings", () => {
    expect(argsToArrayOrUndefined(" one\r\n\r\n two ")).toEqual(["one", "two"]);
  });

  it("normalizes HTTP header keys before creating payload records", () => {
    expect(
      headerEntriesToRecordOrUndefined([
        [" Authorization ", "Bearer token"],
        ["X-Trace", "abc"],
      ]),
    ).toEqual({ authorization: "Bearer token", "x-trace": "abc" });
    expect(headerEntriesToRecordOrNull([["", "ignored"]])).toBeNull();
  });

  it("reports every row that participates in a duplicate trimmed key", () => {
    expect(
      Array.from(
        findDuplicateKeys([
          [" TOKEN ", "a"],
          ["TOKEN", "b"],
          ["OTHER", "c"],
        ]),
      ),
    ).toEqual([0, 1]);
  });

  it("keeps environment keys case-sensitive but reports duplicate HTTP headers case-insensitively", () => {
    expect(
      Array.from(
        findDuplicateKeys([
          ["TOKEN", "a"],
          ["token", "b"],
        ]),
      ),
    ).toEqual([]);
    expect(
      Array.from(
        findDuplicateHeaderKeys([
          [" Authorization ", "Bearer a"],
          ["authorization", "Bearer b"],
          ["X-Other", "c"],
        ]),
      ),
    ).toEqual([0, 1]);
  });

  it("returns shared typed update payloads", () => {
    const stdioUpdates: ServerUpdateInput = formToUpdates(
      {
        name: " Local ",
        command: " node ",
        url: "",
        args: " --stdio \n",
        env: [["TOKEN", "secret"]],
        headers: [],
        workingDir: " /tmp/moor ",
        instructions: " Use read-only tools ",
      },
      "stdio",
    );
    expect(stdioUpdates).toEqual({
      name: "Local",
      command: "node",
      args: ["--stdio"],
      env: { TOKEN: "secret" },
      workingDir: "/tmp/moor",
      instructions: "Use read-only tools",
    });

    const httpUpdates: ServerUpdateInput = formToUpdates(
      {
        name: " Remote ",
        command: "",
        url: " https://example.com/mcp ",
        args: "",
        env: [],
        headers: [[" Authorization ", "Bearer token"]],
        workingDir: "",
      },
      "http",
    );
    expect(httpUpdates).toEqual({
      name: "Remote",
      url: "https://example.com/mcp",
      headers: { authorization: "Bearer token" },
      env: null,
      instructions: "",
    });
  });
});
