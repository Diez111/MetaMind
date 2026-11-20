---
materia: Materia Ejemplo
anio: 1
tipo: materia
curso: C-01 · 100
fecha: 2026-11-09
tags: [materia/ejemplo, anio/1, tipo/materia]
institucion: Universidad Ejemplo
---
# Materia Ejemplo

> [!abstract] Empezá acá
> Ejemplo anonimizado de una nota de materia del vault real.

```dataview
TABLE tipo AS "Tipo", unidad AS "Unidad", file.mtime AS "Editado"
FROM "anon"
WHERE tipo != "materia"
SORT unidad ASC, file.name ASC
```
