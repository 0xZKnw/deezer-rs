# deezer-rs

Lecteur Deezer de bureau, non officiel et léger, pour macOS. Écrit en Rust avec egui (glow) et le lecteur officiel du site intégré dans une WebView.

> Projet perso, sans lien avec Deezer. Il faut un compte Deezer. Aucun contournement de DRM : les morceaux complets sont joués par le lecteur officiel de deezer.com, selon ton abonnement.

## Installer

Télécharge `Deezer-macos-universal.zip` depuis les [Releases](https://github.com/0xZKnw/deezer-rs/releases), dézippe-le, puis glisse `Deezer.app` dans Applications.

L'app est signée « ad hoc », sans compte développeur Apple. La première fois, fais clic droit → **Ouvrir**, ou lance :

```bash
xattr -dr com.apple.quarantine /Applications/Deezer.app
```

Les mises à jour suivantes se font depuis l'app : **Avatar → Settings → Updates**. L'archive téléchargée est vérifiée par son empreinte SHA-256, puis l'app se relance.

## Compiler

```bash
./bundle.sh && open target/Deezer.app      # build pour cette machine
UNIVERSAL=1 ./bundle.sh                     # binaire universel + dist/Deezer-macos-universal.zip
```

## Publier une version

1. Monte `version` dans `Cargo.toml`.
2. Commit, puis lance :

   ```bash
   git tag v0.2.0 && git push origin main v0.2.0
   ```

3. La CI (`.github/workflows/release.yml`) construit l'app universelle et crée la release.
4. Les apps installées la proposent au lancement.

## Connexion

Utilise **Log in to Deezer**. La vraie page de connexion Deezer s'ouvre dans l'app, avec email + mot de passe, Google ou Apple. L'app ne voit jamais ton mot de passe.

Ta session est gardée localement, dans `~/Library/Application Support/deezer-rs/config.json` (droits 0600). Ce fichier n'est jamais dans le dépôt.

Il y a aussi deux autres modes :
- un **profil public**, avec des extraits de 30 s ;
- l'**OAuth Deezer**, avec ta propre app sur developers.deezer.com.

## Design

- Fond graphite avec une lueur ambiante (petit shader GPU) qui prend les couleurs de la pochette en cours.
- La couleur d'accent de l'interface (boutons, progression, morceau en cours) suit la pochette.
- Typo SF Pro en graisses variables.
- Lecteur flottant en « dock ».

## Optimisations

- Repaint seulement sur événement. La lueur est animée à environ 15 fps, uniquement pendant la lecture et quand la fenêtre est active. Sinon elle est figée et ne coûte rien.
- Les polices système sont projetées en mémoire (mmap) au lieu d'être copiées sur le tas.
- Les listes sont virtualisées. Les textures sont en LRU, bornées à 28 Mo. Le cache disque des images est borné.
- Pas de runtime async : `ureq` bloquant sur quelques threads.
- Build en LTO fat, avec `panic=abort` et `strip`.

## Raccourcis

`Espace` lecture/pause · `⌘←` / `⌘→` précédent/suivant.
