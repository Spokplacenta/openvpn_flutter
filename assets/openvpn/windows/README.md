# OpenVPN binaire embarqué (Windows)

Dépose ici les binaires runtime OpenVPN utilisés par `WindowsBinaryManager`.

Étapes recommandées :
1. Récupère les fichiers signés depuis une installation officielle OpenVPN (dossier `bin`).
2. Dépose les fichiers suivants dans ce dossier d'assets :
   - `openvpn.exe.bin` (ou `openvpn.exe`)
   - `libcrypto-3-x64.dll` (ou `libcrypto-3-x64.dll.bin`)
   - `libssl-3-x64.dll` (ou `libssl-3-x64.dll.bin`)
   - `libpkcs11-helper-1.dll` (ou `libpkcs11-helper-1.dll.bin`)
3. Mets à jour WindowsBinaryManager.targetVersion et expectedHash pour refléter cette version.
4. Vérifie les obligations de la licence GPLv2 avant distribution.

Le plugin déploie automatiquement ces DLL à côté de `openvpn.exe` au premier lancement.

Ce dépôt peut contenir uniquement ce fichier de documentation selon la branche. Ajoute les binaires avant distribution.
