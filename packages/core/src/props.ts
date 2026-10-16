// Типы пропсов, описываемые в рантайме: из одного описания получаются TS-тип (в спеке),
// JSON Schema для пульта и `keyfill schema`, проверка значений из команд и приведение строк `--set`.

import type * as S from '@keyfill/core';

/** Описание пропса в рантайме. `String`, `Number`, `Boolean` — короткая запись первых трёх. */
export type PropDesc =
  | { $prop: 'string'; default?: unknown; description?: string }
  | { $prop: 'number'; min?: number; max?: number; int?: boolean; default?: unknown; description?: string }
  | { $prop: 'boolean'; default?: unknown; description?: string }
  | { $prop: 'enum'; values: readonly string[]; default?: unknown; description?: string }
  | { $prop: 'list'; item: PropDesc; min?: number; max?: number; default?: unknown; description?: string }
  | { $prop: 'object'; fields: Record<string, PropDesc>; default?: unknown; description?: string }
  | { $prop: 'image'; default?: unknown; description?: string }
  | { $prop: 'json'; default?: unknown; description?: string };

/** Пропс сцены, компонента или параметр команды — для ядра, `keyfill schema` и `/schema`. */
export type PropInfo = { name: string; type: string; schema: Record<string, unknown>; default?: unknown; required?: boolean };

const brand = <T>(v: unknown): T => v as T;
const make = (kind: string, o: object = {}, extra: object = {}) => brand<never>({ $prop: kind, ...o, ...extra });

export const prop: typeof S.prop = {
  string: (o?: object) => make('string', o),
  number: (o?: object) => make('number', o),
  boolean: (o?: object) => make('boolean', o),
  enum: (values: readonly string[], o?: object) => {
    if (!Array.isArray(values) || !values.length || values.some((v) => typeof v !== 'string')) {
      throw new TypeError('prop.enum: ожидается непустой массив строк');
    }
    return make('enum', o, { values: [...values] });
  },
  list: (item: unknown, o?: object) => make('list', o, { item: desc(item, 'prop.list') }),
  object: (fields: Record<string, unknown>, o?: object) => make('object', o, { fields: descAll(fields, 'prop.object') }),
  image: (o?: object) => make('image', o),
  json: (o?: object) => make('json', o),
} as unknown as typeof S.prop;

/** Короткая запись и описание — к одному виду. */
export function desc(v: unknown, where: string): PropDesc {
  if (v === String) return { $prop: 'string' };
  if (v === Number) return { $prop: 'number' };
  if (v === Boolean) return { $prop: 'boolean' };
  if (v && typeof v === 'object' && typeof (v as PropDesc).$prop === 'string') return v as PropDesc;
  throw new TypeError(`${where}: тип пропса — String, Number, Boolean или prop.*(); prop<T>() заменён на prop.json<T>()`);
}

export function descAll(schema: Record<string, unknown> | undefined, where: string): Record<string, PropDesc> {
  const out: Record<string, PropDesc> = {};
  for (const k of Object.keys(schema ?? {})) out[k] = desc(schema![k], `${where} ${k}`);
  return out;
}

export const hasDefault = (d: PropDesc) => 'default' in d && d.default !== undefined;

/** Короткая подпись типа для текста: `enum home|away`, `number 0..99`, `list<string>`. */
export function label(d: PropDesc): string {
  switch (d.$prop) {
    case 'number': {
      const kind = d.int ? 'integer' : 'number';
      return d.min === undefined && d.max === undefined ? kind : `${kind} ${d.min ?? ''}..${d.max ?? ''}`;
    }
    case 'enum':
      return `enum ${d.values.join('|')}`;
    case 'list':
      return `list<${label(d.item)}>`;
    case 'object':
      return `object{${Object.keys(d.fields).join(', ')}}`;
    default:
      return d.$prop;
  }
}

/** JSON Schema значения: по ней пульт строит форму. */
export function jsonSchema(d: PropDesc): Record<string, unknown> {
  const s: Record<string, unknown> = (() => {
    switch (d.$prop) {
      case 'string':
        return { type: 'string' };
      case 'number':
        return { type: d.int ? 'integer' : 'number', ...(d.min !== undefined && { minimum: d.min }), ...(d.max !== undefined && { maximum: d.max }) };
      case 'boolean':
        return { type: 'boolean' };
      case 'enum':
        return { type: 'string', enum: [...d.values] };
      case 'list':
        return { type: 'array', items: jsonSchema(d.item), ...(d.min !== undefined && { minItems: d.min }), ...(d.max !== undefined && { maxItems: d.max }) };
      case 'object':
        return objectSchema(d.fields);
      case 'image':
        return { type: 'string', 'x-keyfill': 'image' };
      case 'json':
        return { 'x-keyfill': 'json' };
    }
  })();
  if (hasDefault(d)) s.default = d.default;
  if (d.description) s.description = d.description;
  return s;
}

export function objectSchema(fields: Record<string, PropDesc>): Record<string, unknown> {
  const properties: Record<string, unknown> = {};
  for (const [k, f] of Object.entries(fields)) properties[k] = jsonSchema(f);
  const required = Object.keys(fields).filter((k) => !hasDefault(fields[k]));
  return { type: 'object', properties, ...(required.length && { required }), additionalProperties: false };
}

/** Описание пропсов для ядра. `defaults` — значения по умолчанию сцены (`defineScene({ defaults })`). */
export function infos(fields: Record<string, PropDesc>, defaults: Record<string, unknown> = {}): PropInfo[] {
  return Object.keys(fields).map((name) => {
    const d = fields[name];
    const def = name in defaults ? defaults[name] : d.default;
    const schema = jsonSchema(d);
    if (def !== undefined) schema.default = def;
    return { name, type: label(d), schema, ...(def !== undefined ? { default: def } : { required: true }) };
  });
}

const show = (v: unknown) => JSON.stringify(v) ?? String(v);
const article: Record<string, string> = {
  string: 'строка',
  boolean: 'true или false',
  image: 'путь или URL картинки (строка)',
};

/**
 * Проверяет значение по описанию и возвращает его с подставленными значениями по умолчанию
 * у полей объектов. Ошибки — с путём: `players[1].number: ожидается число ≥ 0, получено -1`.
 */
export function check(d: PropDesc, v: unknown, path: string, errors: string[]): unknown {
  const fail = (expected: string) => {
    errors.push(`${path}: ожидается ${expected}, получено ${show(v)}`);
    return v;
  };
  switch (d.$prop) {
    case 'string':
    case 'image':
      return typeof v === 'string' ? v : fail(article[d.$prop]);
    case 'boolean':
      return typeof v === 'boolean' ? v : fail(article.boolean);
    case 'number': {
      const range = [d.min !== undefined && `≥ ${d.min}`, d.max !== undefined && `≤ ${d.max}`].filter(Boolean).join(' и ');
      const expected = `${d.int ? 'целое число' : 'число'}${range ? ` ${range}` : ''}`;
      if (typeof v !== 'number' || !Number.isFinite(v)) return fail(expected);
      if (d.int && !Number.isInteger(v)) return fail(expected);
      if ((d.min !== undefined && v < d.min) || (d.max !== undefined && v > d.max)) return fail(expected);
      return v;
    }
    case 'enum':
      return typeof v === 'string' && d.values.includes(v) ? v : fail(`одно из ${d.values.join(', ')}`);
    case 'list': {
      if (!Array.isArray(v)) return fail('массив');
      if (d.min !== undefined && v.length < d.min) errors.push(`${path}: нужно не меньше ${d.min} элементов, получено ${v.length}`);
      if (d.max !== undefined && v.length > d.max) errors.push(`${path}: можно не больше ${d.max} элементов, получено ${v.length}`);
      return v.map((x, i) => check(d.item, x, `${path}[${i}]`, errors));
    }
    case 'object': {
      if (!v || typeof v !== 'object' || Array.isArray(v)) return fail(`объект с полями ${Object.keys(d.fields).join(', ')}`);
      return fields(d.fields, v as Record<string, unknown>, path || 'значение', errors, true);
    }
    case 'json':
      return v;
  }
}

/**
 * Значения набора полей: неизвестные — ошибка, пропущенные берут значение по умолчанию, без
 * него — ошибка, если `complete` (полный набор); иначе это частичное обновление (`update`).
 * `path` — путь вложенного объекта; на верхнем уровне пустой, и ошибки называют `owner`.
 */
export function fields(
  schema: Record<string, PropDesc>,
  values: Record<string, unknown>,
  path: string,
  errors: string[],
  complete: boolean,
  defaults: Record<string, unknown> = {},
  owner = 'объекта',
): Record<string, unknown> {
  const known = Object.keys(schema);
  const at = (k: string) => (path ? `${path}.${k}` : k);
  const list = known.length ? `есть: ${known.join(', ')}` : 'пропсов нет';
  for (const k of Object.keys(values)) {
    if (!(k in schema)) errors.push(path ? `${path}: нет поля "${k}"; ${list}` : `у ${owner} нет пропса "${k}"; ${list}`);
  }
  const out: Record<string, unknown> = {};
  for (const k of known) {
    if (k in values && values[k] !== undefined) out[k] = check(schema[k], values[k], at(k), errors);
    else if (!complete) continue;
    else if (k in defaults) out[k] = defaults[k];
    else if (hasDefault(schema[k])) out[k] = schema[k].default;
    else errors.push(`не задан обязательный пропс "${at(k)}" (${label(schema[k])})`);
  }
  return out;
}

/** Строка из `--set name=…` → значение по типу пропса; JSON — для списков, объектов и json. */
export function coerce(d: PropDesc, raw: string): { value?: unknown; error?: string } {
  switch (d.$prop) {
    case 'string':
    case 'image':
    case 'enum':
      return { value: raw };
    case 'number':
      return { value: raw.trim() === '' ? NaN : Number(raw) };
    case 'boolean':
      return { value: raw === 'true' ? true : raw === 'false' ? false : raw };
    default:
      try {
        return { value: JSON.parse(raw) };
      } catch {
        return { error: `ожидается JSON (${label(d)}), получено ${raw}` };
      }
  }
}
