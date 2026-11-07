import type { ConnectionType, ServerDetail, ServerUpdateInput } from "@moor/types";

export type KeyValueEntries = Array<[string, string]>;

function entriesToRecord(
  entries: KeyValueEntries,
  normalizeKey: (key: string) => string = (key) => key.trim(),
): Record<string, string> {
  const record: Record<string, string> = {};
  for (const [key, value] of entries) {
    const trimmedKey = normalizeKey(key);
    if (trimmedKey) {
      record[trimmedKey] = value;
    }
  }
  return record;
}

export function entriesToRecordOrUndefined(
  entries: KeyValueEntries,
): Record<string, string> | undefined {
  const record = entriesToRecord(entries);
  return Object.keys(record).length > 0 ? record : undefined;
}

export function entriesToRecordOrNull(entries: KeyValueEntries): Record<string, string> | null {
  const record = entriesToRecord(entries);
  return Object.keys(record).length > 0 ? record : null;
}

export function headerEntriesToRecordOrUndefined(
  entries: KeyValueEntries,
): Record<string, string> | undefined {
  const record = entriesToRecord(entries, (key) => key.trim().toLowerCase());
  return Object.keys(record).length > 0 ? record : undefined;
}

export function headerEntriesToRecordOrNull(
  entries: KeyValueEntries,
): Record<string, string> | null {
  const record = entriesToRecord(entries, (key) => key.trim().toLowerCase());
  return Object.keys(record).length > 0 ? record : null;
}

function argsToArray(args: string): string[] {
  return args
    .split(/\r?\n/)
    .map((arg) => arg.trim())
    .filter(Boolean);
}

export function argsToArrayOrUndefined(args: string): string[] | undefined {
  const parsed = argsToArray(args);
  return parsed.length > 0 ? parsed : undefined;
}

export function argsToArrayOrNull(args: string): string[] | null {
  const parsed = argsToArray(args);
  return parsed.length > 0 ? parsed : null;
}

export function findDuplicateKeys(entries: KeyValueEntries): Set<number> {
  const firstByKey = new Map<string, number>();
  const duplicateIndexes = new Set<number>();

  entries.forEach(([key], index) => {
    const trimmedKey = key.trim();
    if (!trimmedKey) return;

    const firstIndex = firstByKey.get(trimmedKey);
    if (firstIndex === undefined) {
      firstByKey.set(trimmedKey, index);
      return;
    }

    duplicateIndexes.add(firstIndex);
    duplicateIndexes.add(index);
  });

  return duplicateIndexes;
}

export function findDuplicateHeaderKeys(entries: KeyValueEntries): Set<number> {
  const firstByKey = new Map<string, number>();
  const duplicateIndexes = new Set<number>();

  entries.forEach(([key], index) => {
    const normalizedKey = key.trim().toLowerCase();
    if (!normalizedKey) return;

    const firstIndex = firstByKey.get(normalizedKey);
    if (firstIndex === undefined) {
      firstByKey.set(normalizedKey, index);
      return;
    }

    duplicateIndexes.add(firstIndex);
    duplicateIndexes.add(index);
  });

  return duplicateIndexes;
}

export interface EditForm {
  name: string;
  command: string;
  url: string;
  args: string;
  env: KeyValueEntries;
  headers: KeyValueEntries;
  workingDir: string;
  instructions?: string;
}

export function serverToForm(server: ServerDetail): EditForm {
  return {
    name: server.name ?? "",
    command: server.command ?? "",
    url: server.url ?? "",
    args: server.args?.join("\n") ?? "",
    env: server.env ? Object.entries(server.env) : [],
    headers: server.headers ? Object.entries(server.headers) : [],
    workingDir: server.workingDir ?? "",
    instructions: server.instructions ?? "",
  };
}

export function validateEditForm(form: EditForm, connectionType: ConnectionType): string | null {
  if (!form.name.trim()) return "Name is required.";
  if (connectionType === "stdio" && !form.command.trim()) return "Command is required.";
  if (connectionType === "http" && !form.url.trim()) return "URL is required.";
  if (findDuplicateKeys(form.env).size > 0) return "Environment variable keys must be unique.";
  if (connectionType === "http" && findDuplicateHeaderKeys(form.headers).size > 0) {
    return "HTTP header keys must be unique.";
  }
  return null;
}

export function formToUpdates(form: EditForm, connectionType: ConnectionType): ServerUpdateInput {
  const updates: ServerUpdateInput = { name: form.name.trim(), instructions: form.instructions?.trim() ?? "" };
  const env = entriesToRecordOrNull(form.env);

  if (connectionType === "stdio") {
    updates.command = form.command.trim();
    updates.args = argsToArrayOrNull(form.args);
    updates.env = env;
    updates.workingDir = form.workingDir.trim() || null;
    return updates;
  }

  updates.url = form.url.trim();
  updates.headers = headerEntriesToRecordOrNull(form.headers);
  updates.env = env;
  return updates;
}

function stableEntries(
  entries: KeyValueEntries,
  normalizeKey: (key: string) => string = (key) => key.trim(),
): KeyValueEntries {
  return entries
    .map(([key, value]) => [normalizeKey(key), value] as [string, string])
    .filter(([key]) => key)
    .sort(([a], [b]) => a.localeCompare(b));
}

function stableArgs(args: string): string[] {
  return args
    .split(/\r?\n/)
    .map((arg) => arg.trim())
    .filter(Boolean);
}

export function hasChanges(form: EditForm, baseline: EditForm): boolean {
  return (
    form.name.trim() !== baseline.name.trim() ||
    form.command.trim() !== baseline.command.trim() ||
    form.url.trim() !== baseline.url.trim() ||
    form.workingDir.trim() !== baseline.workingDir.trim() ||
    (form.instructions ?? "").trim() !== (baseline.instructions ?? "").trim() ||
    JSON.stringify(stableArgs(form.args)) !== JSON.stringify(stableArgs(baseline.args)) ||
    JSON.stringify(stableEntries(form.env)) !== JSON.stringify(stableEntries(baseline.env)) ||
    JSON.stringify(stableEntries(form.headers, (key) => key.trim().toLowerCase())) !==
      JSON.stringify(stableEntries(baseline.headers, (key) => key.trim().toLowerCase()))
  );
}
