import { jsx } from './jsx-runtime';

export { Fragment } from './jsx-runtime';

type Source = { fileName: string; lineNumber: number; columnNumber: number };

// Dev-рантайм добавляет к пропсам место в исходнике: из него ядро строит provenance и ошибки.
export function jsxDEV(type: any, props: Record<string, any>, _key: unknown, _isStatic: boolean, source?: Source) {
  if (source) props.__src = `${source.fileName}:${source.lineNumber}:${source.columnNumber}`;
  return jsx(type, props);
}
