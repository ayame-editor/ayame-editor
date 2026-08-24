// Inverts the state -> i18n dependency without teaching the translation
// catalog about the application's mutable state shape.
let readLanguagePreference: () => unknown = () => "auto";

export function languagePreference(): unknown {
  return readLanguagePreference();
}

export function setLanguagePreferenceReader(reader: () => unknown) {
  readLanguagePreference = reader;
}
