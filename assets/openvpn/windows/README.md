# OpenVPN binaire embarqué

Dépose ici l'archive binaire \openvpn.exe.bin\ (issue d'une version officielle OpenVPN). Ce fichier est chargé par \WindowsBinaryManager\ puis extrait vers le répertoire d'assistance de l'application.

Étapes recommandées :
1. Récupère l'exécutable OpenVPN signé (par ex. openvpn.exe depuis l'installeur officiel).
2. Renomme-le en openvpn.exe.bin sans transformation supplémentaire. Tu peux aussi le compresser toi-même mais pense à mettre à jour la logique d'extraction si nécessaire.
3. Mets à jour WindowsBinaryManager.targetVersion et expectedHash pour refléter cette version.
4. Vérifie les obligations de la licence GPLv2 avant distribution.

Ce dépôt contient uniquement un fichier factice pour permettre la compilation. Remplace-le avant de distribuer ton application.
