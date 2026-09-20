# Redactio – Windows-Kurzstart

**Vorabstand:** Die vollständige Windows-Freigabe einschließlich Prüfung, Export
und privater Qualitätsbewertung steht noch aus. Diese Anleitung beschreibt den
vorgesehenen vollständigen Arbeitsablauf; ein frühes Prüfpaket bestätigt ihn
noch nicht vollständig.

## 1. Entpacken und starten

Benötigt wird Windows 11 (64 Bit). Das ZIP vollständig in einen lokalen Ordner
im eigenen Benutzerverzeichnis entpacken und darin `redactio.exe` öffnen.
Nicht direkt aus dem ZIP oder von einem Netzlaufwerk starten. Administratorrechte
und eine separate Installation von Python, Node oder WebView2 sind nicht nötig.

Die Ordner `sidecar`, `models` und `webview2` müssen neben `redactio.exe` bleiben.
Die Verarbeitung erfolgt lokal; Modelle werden nicht aus dem Internet nachgeladen.
Bei fehlenden oder beschädigten Ressourcen das vollständige Original-ZIP erneut
in einen **neuen** lokalen Ordner entpacken. Eigene Quellen, Arbeitsausgaben und
Einstellungen dabei nicht überschreiben. Bei einem fehlenden oder inkompatiblen
Modell ein vorhandenes kompatibles Modell wählen oder das vollständige Paket
wiederherstellen; es gibt keinen automatischen Modelldownload.

## 2. Private Ordner und Sammlung einrichten

Für jede Sammlung ein benanntes Ordnerpaar anlegen:

- **Quelle:** die privaten DOCX-Dateien, auch in Unterordnern. Redactio verändert
  die Originaldokumente nicht, benötigt dort aber Schreibrechte für Zuordnung und
  Prüfdaten.
- **Arbeitsausgabe:** ein eigener privater Ordner für `doc-0001.md` usw. Er kann
  ungeprüfte oder noch identifizierende Inhalte enthalten und ist kein Exportordner.

Lokale Ordner außerhalb von Cloud-Synchronisierung verwenden. Quelle und
Arbeitsausgabe dürfen weder gleich sein noch ineinander liegen; das gilt auch
gegenüber anderen gespeicherten Paaren. Verknüpfungen, Windows-Junctions und
andere Schreibweisen heben diese Trennung nicht auf. Für einen vorhandenen,
nicht leeren Ausgabeordner die Eigentums- und Konfliktmeldungen beachten;
unbekannte Dateien werden nicht pauschal ersetzt. Einen fehlenden Zielordner nur
nach eigener Bestätigung anlegen lassen.

Das aktive Paar ist jederzeit an der Paarauswahl erkennbar. Vor Verarbeitung,
Einstellungen, Prüfung oder Export den Sammlungsnamen kontrollieren. Die Auswahl
wird beim nächsten Start wiederhergestellt. Paare lassen sich benennen,
umbenennen, auswählen und nach Bestätigung entfernen. **Entfernen löscht nur die
Konfiguration:** Originale, Zuordnung, Prüfdaten, Arbeitsausgaben und Exporte bleiben.

Gespeicherte Ordnerpfade werden nicht nachträglich verschoben. Für andere Ordner
ein weiteres Paar anlegen. Wird eine bereits verwaltete Quelle erneut hinzugefügt,
gilt ihre gespeicherte Bindung an den bisherigen Ausgabeordner. Fehlende frühere
Erkennungseinstellungen können Ergebnisse veralten lassen; geprüfte Arbeit wird
deshalb nicht stillschweigend ersetzt.

## 3. Erkennung einstellen

Die Einstellungen gehören jeweils zum ausgewählten Paar. Standardmodell ist
**BiomedBERT**. Seine Modelltypen werden aus dem installierten Modell geladen,
zurzeit 54 Typen wie Vorname (FIRSTNAME), Nachname (LASTNAME) und Postleitzahl
(ZIPCODE). Neue Paare aktivieren alle Modelltypen. Zusätzliche Erkennung für
E-Mail-Adressen, Telefonnummern, IBANs, IP-Adressen, URLs und Datum/Zeit lässt
sich getrennt einstellen. Die ursprünglichen Modelltypen bleiben in Prüfung und
Platzhaltern erhalten.

Bei bestehenden BiomedBERT-Paaren die Modelltypen prüfen, speichern und Dokumente
erneut verarbeiten. Bis zum Speichern bleibt die bisherige Erkennung aktiv.
Core-news-Modelle stehen nicht mehr zur Auswahl; betroffene Paare auf BiomedBERT
umstellen. Frühere Ergebnisse werden dabei nicht stillschweigend überschrieben.
Für eine Entwicklungsinstallation zuerst die [Modelleinrichtung](development.md#offline-german-model)
durchführen. Auch BiomedBERT kann Namen und Adressbestandteile übersehen.

Wenn zusätzlich eingerichtet, steht **HuggingLil – Deutsch, PII** als zweites
Modell zur Auswahl. Es verwendet eigene Typen wie GIVENNAME, SURNAME und CITY.
Das Modell pro Ordnerpaar auswählen, die gewünschten Typen prüfen, speichern und
vorhandene Dokumente erneut verarbeiten. Die bisherigen Ergebnisse werden nicht
automatisch umgeschrieben. Die [Einrichtung der Alternative](development.md#hugginglil-alternative)
erfolgt ausdrücklich vor der lokalen Nutzung.

Benötigte Kategorien aktivieren und bei Bedarf eigene Wortlisten oder reguläre
Ausdrücke ergänzen. Wortlisten suchen wörtlich und unterscheiden Groß-/Kleinschreibung;
reguläre Ausdrücke verwenden Python-Syntax. Regeln zuerst mit erfundenem
Beispieltext in der Vorschau prüfen. Fehlerhafte Regeln oder eine abgelaufene
Vorschau korrigieren, bevor die Konfiguration gespeichert wird. Eigene Begriffe,
Muster und Vorschautexte bleiben private Daten.

Die Ausgabeoption für detaillierte Schwärzungspositionen betrifft die
Markdown-Metadaten; die interne Prüfung behält ihre benötigten Positionen.
Änderungen an Modell, Erkennung, Regeln oder Ausgabeoptionen machen bisherige
Ergebnisse dieses Paars veraltet. Auch ein Verarbeitungsupdate kann erneute
Verarbeitung und Prüfung erfordern. Das bloße Auswählen oder Umbenennen tut dies nicht.

Über **Erscheinungsbild** im Kopfbereich zwischen **Dunkel**, **Hell** und **Wie
Windows** wählen. Die Auswahl bleibt beim nächsten Start erhalten. Ohne gespeicherte
Auswahl startet Redactio im dunklen Onyx-Design.

## 4. Dokumente verarbeiten und Fehler behandeln

Unter **Dokumente** zuerst **Quellordner einlesen** wählen. Die Übersicht zeigt
gefundene DOCX-Dateien und ihren Zustand. Vor dem Start auch Such- und Lesefehler
prüfen. Versteckte Dateien/Ordner, Word-Sperrdateien
(`~$`), App-Metadaten und verknüpfte Unterordner gehören nicht zur Verarbeitung.
PDF, gescannte Bilder und OCR werden nicht unterstützt.

Mit **Verarbeitung starten** das ausgewählte Paar verarbeiten und Fortschritt sowie
Abschlussübersicht beachten. Es läuft jeweils nur ein Paar; während einer
Operation sind Paarwechsel und Konfigurationsänderungen gesperrt. Unveränderte,
erfolgreiche Ergebnisse werden übersprungen. Fehlende Ausgaben können neu erzeugt
werden; geänderte Quellen oder Einstellungen erfordern neue Ergebnisse.

**Abbrechen** beendet die weitere Planung, sobald die laufende begrenzte
Operation beendet oder abgebrochen ist. Bereits gespeicherte Ergebnisse bleiben
erhalten. Die Übersicht unterscheidet verarbeitet, übersprungen, fehlgeschlagen
und noch nicht verarbeitet. Nach Behebung der Ursache fehlgeschlagene Dateien
über **Fehlgeschlagene Dokumente erneut versuchen** gezielt erneut versuchen.
Nach der Verarbeitung aktualisiert sich die Dokumentliste automatisch. Über
**doc-… prüfen** ein aktuelles Ergebnis direkt öffnen; ein erneutes Einlesen ist
nicht nötig. Auch nach dem Speichern einer Prüfung werden die Zustände automatisch
aktualisiert, und die Auswahl **Dokument prüfen** bleibt verfügbar. Eine Zeitüberschreitung ist ein Fehler, kein Erfolg;
die Initialisierung ist auf 180 Sekunden, eine Dokumentanfrage auf 120 Sekunden begrenzt.

Arbeitsausgaben nicht außerhalb von Redactio bearbeiten: geänderte Ausgabedateien
führen zu einem Konflikt. Vor bewusstem Neuverarbeiten geprüfter oder korrigierter
Dokumente kontrollieren, welche Entscheidungen verworfen werden, und dies
ausdrücklich bestätigen. Verschobene oder umbenannte Quelldateien erhalten eine
neue Dokumentidentität. Redactio löscht alte Originale oder Ausgaben nicht automatisch.

## 5. Inhalt prüfen und korrigieren

Jedes Ergebnis vor einer Weitergabe fachlich prüfen. Bei einem Dokument mit dem
Zustand **Aktuell** die Schaltfläche **doc-0001 prüfen** (mit der jeweiligen ID)
wählen. In der Prüfungsansicht **Originaltext** und **Geschwärzte Vorschau** vergleichen.
Weitere aktuelle Ergebnisse sind unter **Dokument prüfen** auswählbar.

Übersehene Angaben direkt in der **Geschwärzten Vorschau** markieren, den **Typ
der neuen Schwärzung** wählen und **Auswahl schwärzen** betätigen. Das Original
bleibt zum Vergleich daneben sichtbar. Bereits geschwärzte Angaben erscheinen
als Platzhalter; eine Auswahl über einen Platzhalter umfasst dessen ganze
Originalstelle. Für die Tastatur **Per Tastatur
auswählen** öffnen, mit Umschalt- und Pfeiltasten markieren und mit Umschalt+Tab
zur Schaltfläche **Auswahl schwärzen** zurückkehren. Unter **Schwärzungen
bearbeiten** steht der betroffene Originaltext. **Im Text anzeigen** springt zur
Stelle und hebt sie in beiden Ansichten hervor. Ein Klick auf eine markierte
Stelle öffnet umgekehrt ihren Listeneintrag. Dort falsche Treffer entfernen oder
ihren Typ korrigieren.
**Korrektur zurücknehmen** macht die letzte Schwärzungskorrektur der Sitzung
rückgängig. Allgemeine Textbearbeitung und frei gewählte Ersatztexte sind nicht
Teil dieses Arbeitsablaufs.

Die Vorschau zeigt Schwärzungskorrekturen sofort. Die Ausgabedatei wird erst
beim Speichern geändert.
Zuerst **Prüfung speichern**, dann die aktualisierte Ausgabe kontrollieren und
erst danach ausdrücklich **Freigeben**. Ungespeicherte Schwärzungskorrekturen
sperren die Freigabe. Beim Dokument- oder Paarwechsel, beim Verlassen der Ansicht
oder beim Schließen fragt **Ungespeicherte Prüfung** nach: **Speichern und
fortfahren**, **Änderungen verwerfen** oder **Hier bleiben**. Schlägt das Speichern
fehl, bleiben die Änderungen erhalten und die Ansicht wird nicht verlassen.

Absätze und Tabellen werden in lesbarer Reihenfolge übernommen, nicht das genaue
Word-Layout. Warnungen können unter anderem Kopf-/Fußzeilen, Fuß-/Endnoten,
Textfelder, Kommentare, Änderungsverfolgung, Bilder oder eingebettete Objekte
betreffen. Solche Inhalte im **Original-DOCX** gesondert prüfen; die Textansicht
ist dafür keine vollständige Darstellung. Eine leere Extraktion kann nicht
freigegeben werden. Nicht leere Ergebnisse mit Warnungen erst nach deren
ausdrücklicher Bestätigung freigeben.

Die Prüfzustände heißen **Ausstehend**, **Freigegeben**, **Abgelehnt** und
**Nacharbeit erforderlich**. Mit **Als ausstehend speichern** lässt sich eine
Freigabe wieder aufheben. Korrekturen
und Neuerzeugung heben eine frühere Freigabe auf. Geänderte Originale,
Einstellungen oder Ausgabebytes machen alte Freigaben ungültig; alte Markierungen
werden nicht ungeprüft auf neuen Text übertragen. **Prüfnotizen bleiben privat**
bei den Quelldaten und werden nicht exportiert.

Erkennungswerte sind keine Zusicherung von Datenschutz. Auch ein mehrfach
vorkommender Name kann an einer Stelle übersehen werden; Kontext kann weiterhin
identifizieren. Pseudonymisierung und Freigabe bieten **keine garantierte Anonymität**
und bestätigen keine Berechtigung zur Weitergabe.

## 6. Freigegebene Ergebnisse exportieren

Unter **Dokumente** die gewünschten freigegebenen Dokumente in der Spalte
**Auswahl** markieren und **Auswahl freigegeben exportieren** wählen. Im Dialog
den Paarnamen und die aufgeführten Dokument-IDs kontrollieren. Die Auswahl bleibt
für diesen Export fest; Paarwechsel und andere Vorgänge sind bis zum Schließen
des Dialogs gesperrt.

Mit **Exportordner auswählen und exportieren** einen eigenen **leeren
Exportordner** wählen. Er darf keine Quelle, Arbeitsausgabe oder App-Einstellungen
enthalten und auch nicht darin liegen. Für jedes Paar einen getrennten Export
verwenden: Dokumentnamen wie `doc-0001.md` können sich zwischen Paaren wiederholen.
**Abbrechen** vor der Ordnerwahl oder der Abbruch der Ordnerauswahl kopiert keine
Dokumente. Während der anschließenden Prüfung und Kopie ist der Dialog gesperrt;
es gibt keinen Abbruch einer bereits laufenden Kopie.

Exportiert werden nur die ausgewählten, aktuell freigegebenen Markdown-Dateien.
Originale, Zuordnung, Notizen und private Regeln werden nicht mitkopiert. Bei
veralteten, fehlenden oder extern veränderten Dateien zunächst den Fehler beheben
und erneut prüfen. Die Ergebnisübersicht unterscheidet **Exportiert** und
**Blockiert oder fehlgeschlagen**. Bei **Export unvollständig** bleiben bereits
exportierte Dateien erhalten; fehlende oder nicht bestätigte Kopien im Exportordner
prüfen. Ein unvollständiger Export ist kein vollständig geprüftes Liefermaterial.
Mit **Schließen** zur Übersicht zurückkehren und für einen weiteren Versuch einen
neuen leeren Ordner verwenden; vorhandene Dateien werden nicht überschrieben.

Eine Protokollwarnung kann auch nach erfolgreichem Export oder Abbruch erscheinen.
Sie bedeutet, dass der Vorgang nicht vollständig protokolliert wurde; bereits
exportierte Dateien bleiben erhalten. Die Warnung vor dem Schließen beachten und
Speicherplatz sowie Schreibrechte am Protokollort prüfen.

Ein Export ist eine geprüfte **Momentaufnahme**. Spätere Änderungen oder der
Widerruf einer Freigabe entfernen bereits exportierte Kopien nicht. Vor deren
Weitergabe selbst prüfen, ob sie noch verwendet werden sollen.

## 7. Protokoll, Sicherung und Wiederherstellung

Unter Einstellungen den angezeigten Speicherort des Audit-Protokolls verwenden
und bei Bedarf dessen Ordner öffnen. `audit-log.jsonl` liegt bei `settings.json`
im App-Konfigurationsverzeichnis, unter Windows üblicherweise
`%APPDATA%\com.redactio.app`. Maßgeblich ist der in der App angezeigte Pfad.
Das Protokoll enthält technische Metadaten zu Läufen und Exporten, keine
Originaltexte, Dateinamen oder privaten Regeln. Meldet die Abschlussübersicht
einen Protokollfehler, bleiben bereits gespeicherte Dokumente erhalten, aber der
Vorgang ist möglicherweise nicht vollständig protokolliert. Speicherplatz und
Schreibrechte prüfen; die Warnung nicht als erfolgreichen Protokolleintrag behandeln.

Bei geschlossener App die Quellen **einschließlich** `_document-mapping.json`
und `_redactio/` mit den Prüfdaten sichern. Auch Arbeitsausgaben und private
App-Einstellungen in eine geschützte Sicherung aufnehmen. Diese Metadaten nicht
löschen, um einen Fehler zu umgehen: Sie verbinden Originale, IDs und Freigaben.
Bei beschädigter oder fehlender Zuordnung eine passende Sicherung wiederherstellen
oder den ausdrücklich bestätigten Neustart mit einem neuen leeren Ausgabeordner
verwenden. Vorhandene Ausgaben nicht als unverwaltete Dateien überschreiben;
angelegte private Wiederherstellungssicherungen behalten.

## 8. Eigene Sammlung lokal bewerten

Originale, identifizierende Screenshots und private Prüfnotizen weder hochladen
noch in ein Quellcode-Repository oder CI übernehmen. Für die Freigabebewertung
mindestens 10 % einer repräsentativen Sammlung sowie **jeden Warnungs- und
Fehlerfall** unabhängig lokal kontrollieren. Übersehene Angaben und falsche
Treffer nach Kategorie zählen, Korrekturen und erneute Prüfung dokumentieren
und verbleibende Grenzen festhalten, ohne Inhalte offenzulegen. Das ersetzt
nicht die Prüfung jedes Dokuments vor seiner Weitergabe. Eine schnelle
synthetische Testverarbeitung belegt weder Qualität noch Vollständigkeit der
Erkennung in der eigenen Sammlung.
