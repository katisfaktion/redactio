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

Die Einstellungen gehören jeweils zum ausgewählten Paar. Standardmäßig werden
Personen, Orte, E-Mail-Adressen, Telefonnummern, IBANs, IP-Adressen, URLs und
Datums-/Zeitangaben mit dem mitgelieferten deutschen Modell `de_core_news_lg`
gesucht. Nur lokal vorhandene kompatible Modelle stehen zur Auswahl.

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

## 4. Dokumente verarbeiten und Fehler behandeln

Die Dokumentübersicht zeigt gefundene DOCX-Dateien und ihren Zustand. Vor dem
Start auch Such- und Lesefehler prüfen. Versteckte Dateien/Ordner, Word-Sperrdateien
(`~$`), App-Metadaten und verknüpfte Unterordner gehören nicht zur Verarbeitung.
PDF, gescannte Bilder und OCR werden nicht unterstützt.

Die Verarbeitung für das ausgewählte Paar starten und Fortschritt sowie
Abschlussübersicht beachten. Es läuft jeweils nur ein Paar; während einer
Operation sind Paarwechsel und Konfigurationsänderungen gesperrt. Unveränderte,
erfolgreiche Ergebnisse werden übersprungen. Fehlende Ausgaben können neu erzeugt
werden; geänderte Quellen oder Einstellungen erfordern neue Ergebnisse.

**Abbrechen** beendet die weitere Planung, sobald die laufende begrenzte
Operation beendet oder abgebrochen ist. Bereits gespeicherte Ergebnisse bleiben
erhalten. Die Übersicht unterscheidet verarbeitet, übersprungen, fehlgeschlagen
und noch nicht verarbeitet. Nach Behebung der Ursache fehlgeschlagene Dateien
gezielt erneut versuchen. Eine Zeitüberschreitung ist ein Fehler, kein Erfolg;
die Initialisierung ist auf 180 Sekunden, eine Dokumentanfrage auf 120 Sekunden begrenzt.

Arbeitsausgaben nicht außerhalb von Redactio bearbeiten: geänderte Ausgabedateien
führen zu einem Konflikt. Vor bewusstem Neuverarbeiten geprüfter oder korrigierter
Dokumente kontrollieren, welche Entscheidungen verworfen werden, und dies
ausdrücklich bestätigen. Verschobene oder umbenannte Quelldateien erhalten eine
neue Dokumentidentität. Redactio löscht alte Originale oder Ausgaben nicht automatisch.

## 5. Inhalt prüfen und korrigieren

Jedes Ergebnis vor einer Weitergabe fachlich prüfen. In der Prüfungsansicht den
extrahierten Originaltext mit der erzeugten Fassung vergleichen. Übersehene
Angaben im Originaltext markieren und eine Schwärzung hinzufügen; falsche Treffer
verwerfen oder ihren Typ korrigieren. Die Ausgabe wird aus diesen Entscheidungen
neu erzeugt. Allgemeine Textbearbeitung und frei gewählte Ersatztexte sind nicht
Teil dieses Arbeitsablaufs. Änderungen der laufenden Prüfsitzung lassen sich
rückgängig machen; vor einem Wechsel speichern oder ausdrücklich verwerfen.

Absätze und Tabellen werden in lesbarer Reihenfolge übernommen, nicht das genaue
Word-Layout. Warnungen können unter anderem Kopf-/Fußzeilen, Fuß-/Endnoten,
Textfelder, Kommentare, Änderungsverfolgung, Bilder oder eingebettete Objekte
betreffen. Solche Inhalte im **Original-DOCX** gesondert prüfen; die Textansicht
ist dafür keine vollständige Darstellung. Eine leere Extraktion kann nicht
freigegeben werden. Nicht leere Ergebnisse mit Warnungen erst nach deren
ausdrücklicher Bestätigung freigeben.

Ein Dokument ist zunächst offen zur Prüfung (`pending`), freigegeben (`approved`),
zurückgewiesen (`rejected`) oder benötigt Nacharbeit (`needs-rework`). Korrekturen
und Neuerzeugung heben eine frühere Freigabe auf. Geänderte Originale,
Einstellungen oder Ausgabebytes machen alte Freigaben ungültig; alte Markierungen
werden nicht ungeprüft auf neuen Text übertragen. **Prüfnotizen bleiben privat**
bei den Quelldaten und werden nicht exportiert.

Erkennungswerte sind keine Zusicherung von Datenschutz. Auch ein mehrfach
vorkommender Name kann an einer Stelle übersehen werden; Kontext kann weiterhin
identifizieren. Pseudonymisierung und Freigabe bieten **keine garantierte Anonymität**
und bestätigen keine Berechtigung zur Weitergabe.

## 6. Freigegebene Ergebnisse exportieren

Die aktuellen freigegebenen Dokumente eines Paars auswählen und einen eigenen
**leeren Exportordner** festlegen. Er muss außerhalb aller Quellen,
Arbeitsausgaben und App-Einstellungen liegen. Für jedes Paar einen getrennten
Export verwenden: Dokumentnamen wie `doc-0001.md` können sich zwischen Paaren wiederholen.

Exportiert werden nur die ausgewählten, aktuell freigegebenen Markdown-Dateien.
Originale, Zuordnung, Notizen und private Regeln werden nicht mitkopiert. Bei
veralteten, fehlenden oder extern veränderten Dateien zunächst den Fehler beheben
und erneut prüfen. Die Ergebnisübersicht auch auf fehlgeschlagene Kopien prüfen;
ein unvollständiger Export ist nicht vollständig freigegebenes Liefermaterial.
Für einen erneuten Export einen neuen leeren Ordner verwenden.

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
