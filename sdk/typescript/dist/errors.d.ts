export interface ConexErrorFields {
    code?: number;
    diagnosticId?: string;
    execution?: string;
    retry?: string;
    details?: unknown;
    status?: number;
}
export declare class ConexError extends Error {
    readonly code?: number;
    readonly diagnosticId?: string;
    readonly execution?: string;
    readonly retry?: string;
    readonly details?: unknown;
    readonly status?: number;
    constructor(message: string, fields?: ConexErrorFields);
}
export declare function toConexError(error: unknown): ConexError;
