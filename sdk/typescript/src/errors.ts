//! Typed conex errors. HTTP protocol errors keep their status separately.
export interface ConexErrorFields {
  code?: number;
  diagnosticId?: string;
  execution?: string;
  retry?: string;
  details?: unknown;
  status?: number;
}

export class ConexError extends Error {
  readonly code?: number;
  readonly diagnosticId?: string;
  readonly execution?: string;
  readonly retry?: string;
  readonly details?: unknown;
  readonly status?: number;

  constructor(message: string, fields: ConexErrorFields = {}) {
    super(message);
    this.name = "ConexError";
    this.code = fields.code;
    this.diagnosticId = fields.diagnosticId;
    this.execution = fields.execution;
    this.retry = fields.retry;
    this.details = fields.details;
    this.status = fields.status;
  }
}

export function toConexError(error: unknown): ConexError {
  if (error instanceof ConexError) {
    return error;
  }
  if (error instanceof Error) {
    return new ConexError(error.message, { details: error.name });
  }
  return new ConexError("unknown error", { details: String(error) });
}
