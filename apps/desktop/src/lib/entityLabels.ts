export const supplementaryEntities = [
  "EMAIL_ADDRESS", "PHONE_NUMBER", "IBAN_CODE", "IP_ADDRESS", "URL", "DATE_TIME",
] as const;

const labels: Record<string, string> = {
  PERSON: "Person", LOCATION: "Ort oder Adresse", EMAIL_ADDRESS: "E-Mail-Adresse",
  PHONE_NUMBER: "Telefonnummer", IBAN_CODE: "IBAN", IP_ADDRESS: "IP-Adresse",
  URL: "Webadresse", DATE_TIME: "Datum und Uhrzeit", CUSTOM: "Benutzerdefiniert",
  ACCOUNTNAME: "Kontoname", ACCOUNTNUM: "Kontonummer", AGE: "Alter", AMOUNT: "Betrag", BANKACCOUNT: "Bankkonto", BIC: "BIC",
  BITCOINADDRESS: "Bitcoin-Adresse", BUILDINGNUM: "Hausnummer", BUILDINGNUMBER: "Hausnummer", CITY: "Stadt", COUNTY: "Landkreis",
  CREDITCARD: "Kreditkarte", CREDITCARDISSUER: "Kreditkartenanbieter", CURRENCY: "Währung",
  CREDITCARDNUMBER: "Kreditkartennummer", CURRENCYCODE: "Währungscode", CURRENCYNAME: "Währungsname", CURRENCYSYMBOL: "Währungssymbol",
  CVV: "Kartenprüfnummer", DATE: "Datum", DATEOFBIRTH: "Geburtsdatum", DRIVERLICENSENUM: "Führerscheinnummer", EMAIL: "E-Mail-Adresse",
  ETHEREUMADDRESS: "Ethereum-Adresse", ETHN: "Ethnische Zugehörigkeit", EYECOLOR: "Augenfarbe", FIRSTNAME: "Vorname", GIVENNAME: "Vorname", GENDER: "Geschlecht",
  GPSCOORDINATES: "GPS-Koordinaten", HEIGHT: "Körpergröße", IBAN: "IBAN", IMEI: "IMEI", IPADDRESS: "IP-Adresse",
  JOBDEPARTMENT: "Abteilung", JOBTITLE: "Berufsbezeichnung", LASTNAME: "Nachname", LITECOINADDRESS: "Litecoin-Adresse",
  IDCARDNUM: "Ausweisnummer", MACADDRESS: "MAC-Adresse", MASKEDNUMBER: "Maskierte Nummer", MIDDLENAME: "Zweiter Vorname", OCCUPATION: "Beruf",
  ORDINALDIRECTION: "Himmelsrichtung", ORGANIZATION: "Organisation", PASSWORD: "Passwort", PHONE: "Telefonnummer", REL: "Religion",
  PIN: "PIN", PREFIX: "Anrede", SECONDARYADDRESS: "Adresszusatz", SEX: "Biologisches Geschlecht",
  SOCIALNUM: "Sozialversicherungsnummer", SOR: "Sexuelle Orientierung", SSN: "Sozialversicherungsnummer", STATE: "Bundesland", STREET: "Straße", SURNAME: "Nachname", TAXNUM: "Steuernummer", TELEPHONENUM: "Telefonnummer", TIME: "Uhrzeit", USERAGENT: "Browserkennung",
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
