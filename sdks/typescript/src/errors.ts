export class ContractGateError extends Error {
  constructor(message: string) {
    super(message);
    this.name = this.constructor.name;
  }
}

export class ContractCompileError extends ContractGateError {}

export class ConnectionError extends ContractGateError {}

export class HTTPError extends ContractGateError {
  constructor(
    message: string,
    public readonly status: number,
    public readonly body: unknown,
  ) {
    super(message);
  }
}

export class BadRequestError extends HTTPError {}
export class AuthError extends HTTPError {}
export class NotFoundError extends HTTPError {}
export class ConflictError extends HTTPError {}
export class ValidationFailedError extends HTTPError {}
export class ServerError extends HTTPError {}

type HTTPErrorCtor = new (message: string, status: number, body: unknown) => HTTPError;

export function statusToException(status: number): HTTPErrorCtor {
  if (status === 400) return BadRequestError;
  if (status === 401) return AuthError;
  if (status === 404) return NotFoundError;
  if (status === 409) return ConflictError;
  if (status === 422) return ValidationFailedError;
  if (status >= 500 && status < 600) return ServerError;
  return HTTPError;
}

export function raiseForStatus(status: number, body: unknown): void {
  if (status >= 200 && status < 300) return;
  const Cls = statusToException(status);
  const message = extractErrorMessage(body) ?? `HTTP ${status}`;
  throw new Cls(message, status, body);
}

function extractErrorMessage(body: unknown): string | null {
  if (body !== null && typeof body === 'object' && !Array.isArray(body)) {
    const b = body as Record<string, unknown>;
    for (const key of ['error', 'message', 'detail']) {
      if (typeof b[key] === 'string') return b[key] as string;
    }
  }
  return null;
}
