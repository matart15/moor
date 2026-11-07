export type ConnectionType = "stdio" | "http";
export type ServerStatus = "stopped" | "starting" | "running" | "error";
export type ServerAction = "starting" | "stopping";

export interface Server {
  id: string;
  name: string;
  connectionType: ConnectionType;
  status: ServerStatus;
  autoStart: boolean;
  instructions?: string | null;
  command?: string | null;
  args?: string[] | null;
  url?: string | null;
  env?: Record<string, string> | null;
  headers?: Record<string, string> | null;
  workingDir?: string | null;
  errorMessage?: string | null;
  sortOrder?: number | null;
  createdAt: string;
  updatedAt: string;
}

export interface ServerRuntime {
  id: string;
  name: string;
  connectionType: ConnectionType;
  status: ServerStatus;
  autoStart: boolean;
  instructions?: string | null;
}

export interface ServerDetail extends Server {
  runtime: ServerRuntime;
}

export interface ServerUpdateInput {
  name?: string;
  command?: string;
  args?: string[] | null;
  url?: string;
  env?: Record<string, string> | null;
  headers?: Record<string, string> | null;
  workingDir?: string | null;
  autoStart?: boolean;
  instructions?: string | null;
}
