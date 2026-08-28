export type Locale =
  | 'en'
  | 'th'
  | 'zh-CN'
  | 'hi'
  | 'es'
  | 'ar'
  | 'fr'
  | 'bn'
  | 'pt'
  | 'de'
  | 'ru'
  | 'id'
  | 'it'
  | 'ko'
  | 'pl';

export interface TranslationMap {
  [key: string]: string;
}
