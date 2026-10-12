export class ConexError extends Error {
    code;
    diagnosticId;
    execution;
    retry;
    details;
    status;
    constructor(message, fields = {}) {
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
export function toConexError(error) {
    if (error instanceof ConexError) {
        return error;
    }
    if (error instanceof Error) {
        return new ConexError(error.message, { details: error.name });
    }
    return new ConexError("unknown error", { details: String(error) });
}
//# sourceMappingURL=errors.js.map