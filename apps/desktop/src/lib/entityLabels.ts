export const supplementaryEntities = [
  "EMAIL_ADDRESS", "PHONE_NUMBER", "IBAN_CODE", "IP_ADDRESS", "URL", "DATE_TIME",
] as const;

const labels: Record<string, string> = {
  PERSON: "Person", LOCATION: "Ort oder Adresse", EMAIL_ADDRESS: "E-Mail-Adresse",
  PHONE_NUMBER: "Telefonnummer", IBAN_CODE: "IBAN", IP_ADDRESS: "IP-Adresse",
  URL: "Webadresse", DATE_TIME: "Datum und Uhrzeit", CUSTOM: "Benutzerdefiniert",
  ACCOUNTNAME: "Kontoname", AGE: "Alter", AMOUNT: "Betrag", BANKACCOUNT: "Bankkonto", BIC: "BIC",
  BITCOINADDRESS: "Bitcoin-Adresse", BUILDINGNUMBER: "Hausnummer", CITY: "Stadt", COUNTY: "Landkreis",
  CREDITCARD: "Kreditkarte", CREDITCARDISSUER: "Kreditkartenanbieter", CURRENCY: "Währung",
  CURRENCYCODE: "Währungscode", CURRENCYNAME: "Währungsname", CURRENCYSYMBOL: "Währungssymbol",
  CVV: "Kartenprüfnummer", DATE: "Datum", DATEOFBIRTH: "Geburtsdatum", EMAIL: "E-Mail-Adresse",
  ETHEREUMADDRESS: "Ethereum-Adresse", EYECOLOR: "Augenfarbe", FIRSTNAME: "Vorname", GENDER: "Geschlecht",
  GPSCOORDINATES: "GPS-Koordinaten", HEIGHT: "Körpergröße", IBAN: "IBAN", IMEI: "IMEI", IPADDRESS: "IP-Adresse",
  JOBDEPARTMENT: "Abteilung", JOBTITLE: "Berufsbezeichnung", LASTNAME: "Nachname", LITECOINADDRESS: "Litecoin-Adresse",
  MACADDRESS: "MAC-Adresse", MASKEDNUMBER: "Maskierte Nummer", MIDDLENAME: "Zweiter Vorname", OCCUPATION: "Beruf",
  ORDINALDIRECTION: "Himmelsrichtung", ORGANIZATION: "Organisation", PASSWORD: "Passwort", PHONE: "Telefonnummer",
  PIN: "PIN", PREFIX: "Anrede", SECONDARYADDRESS: "Adresszusatz", SEX: "Biologisches Geschlecht",
  SSN: "Sozialversicherungsnummer", STATE: "Bundesland", STREET: "Straße", TIME: "Uhrzeit", USERAGENT: "Browserkennung",
  USERNAME: "Benutzername", VIN: "Fahrzeug-Identifikationsnummer", VRM: "Kennzeichen", ZIPCODE: "Postleitzahl",
};

const legacyNativeGroups: Record<string, "PERSON" | "LOCATION"> = {
  FIRSTNAME: "PERSON", MIDDLENAME: "PERSON", LASTNAME: "PERSON",
  STREET: "LOCATION", BUILDINGNUMBER: "LOCATION", SECONDARYADDRESS: "LOCATION", ZIPCODE: "LOCATION",
  CITY: "LOCATION", STATE: "LOCATION", COUNTY: "LOCATION", GPSCOORDINATES: "LOCATION", ORDINALDIRECTION: "LOCATION",
};

export function entityLabel(type: string): string { return labels[type] ?? type; }
export function legacyNativeGroup(type: string): "PERSON" | "LOCATION" | undefined { return legacyNativeGroups[type]; }
export function entityOption(type: string): { value: string; label: string } {
  return { value: type, label: labels[type] ? `${labels[type]} (${type})` : type };
}
