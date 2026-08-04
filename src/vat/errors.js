export function vatError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}
