# COMMITS — дисциплина коммитов и push

Модель — Orca-grade: много сфокусированных коммитов, каждый объясняет сам
себя. Живой пример:
https://github.com/stablyai/orca/commit/9ece2730561375979405825462c812167d6bbbc1

## Формат

Conventional Commits: `type(scope): summary`.

- Английский, императив, lowercase subject, без точки в конце.
- Типы: `feat`, `fix`, `docs`, `refactor`, `test`, `ci`, `style`, `chore`.
- Скоупы из репо: `ci`, `cargo`, `workspace`, `readme`, `agents`, `commits`,
  `conveyor`, `skills`, а также имена крейтов (`cli`, `engine`, `tui`, …).

```
fix(workspace): include titi-secrets as a member so its tests run
docs(readme): fix the broken "continue the work" links
ci: run fmt, clippy, and tests on GitHub Actions
```

## Одна забота — один коммит

Каждый коммит меняет ровно одну вещь. Если в описании появляется «и», «плюс»
или второй несвязанный файл — это два коммита. Рефактор отдельно от фичи,
переименование отдельно от логики.

## Тело — проза, не список галочек

Несколько плотных предложений, перенос ~78 колонок. Тело отвечает на:

- **Зачем** — какая проблема/цель, а не пересказ diff.
- **Инвариант** — что теперь гарантируется или защищено.
- **Компромис** — почему так, а не альтернатива.

Без филлера, без «as requested», без «updated code».

## Push

- Прямо в `master`: `git push origin master`.
- Никогда не force-push (`-f` / `--force` / `--force-with-lease` запрещены).
- Origin ушёл вперёд (Beka запушил) → `git fetch origin && git rebase
  origin/master`, затем обычный push. Конфликт ребейза → остановиться и
  сообщить, не ломать историю.

## Шаблон

`.gitmessage` подключён для этого репо (`git config commit.template
.gitmessage`) — subject-конвенция и подсказки для тела.
