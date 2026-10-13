/** An error safe to show to the user, with the HTTP status to return. */
export class AppError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

export const unauthorized = () => new AppError(401, "unauthorized", "Please sign in.");
export const forbidden = (m = "You don't have permission to do that.") => new AppError(403, "forbidden", m);
export const notFound = (m = "Not found.") => new AppError(404, "not_found", m);
