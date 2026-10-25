import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/controls/section_card.dart';
import 'package:claude_bridge_ui/src/i18n.dart';
import 'package:claude_bridge_ui/src/providers/models_provider.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';

const _providers = <String, String>{
  'anthropic': 'Anthropic',
  'codex-oauth': 'ChatGPT / Codex',
  'openai': 'OpenAI API',
  'openai-compat': 'OpenAI-compatible',
  'gemini-oauth': 'Google / Gemini',
  'gemini': 'Gemini API',
};

class ModelsPage extends StatelessWidget {
  const ModelsPage({super.key});

  @override
  Widget build(BuildContext context) {
    final service = context.appServices.modelsService;
    final settings = context.appServices.settingsService.provider;
    return ListenableBuilder(
      listenable: Listenable.merge([service.provider, settings]),
      builder: (context, _) {
        final state = service.provider.state;
        if (state == null) {
          return const Padding(
            padding: EdgeInsets.all(24),
            child: LinearProgressIndicator(),
          );
        }
        final models = state.models
            .where((model) => settings.providerEnabled(model.provider))
            .toList();
        return ListView(
          padding: const EdgeInsets.all(24),
          children: [
            SectionCard(
              title: tr('Модели', 'Models'),
              icon: Icons.smart_toy_outlined,
              trailing: FilledButton.icon(
                onPressed: () => _editModel(context),
                icon: const Icon(Icons.add),
                label: Text(tr('Добавить', 'Add')),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    tr(
                      'Встроенные модели нельзя изменить или удалить. Пользовательские модели хранятся в ${state.filePath}. Модель, с которой UI запускает прокси, выбирается на главной странице.',
                      'Built-in models cannot be edited or deleted. Custom models are stored in ${state.filePath}. The model the UI starts the proxy with is chosen on the Home page.',
                    ),
                    style: TextStyle(color: Theme.of(context).hintColor),
                  ),
                  const SizedBox(height: 12),
                  if (models.isEmpty)
                    Text(
                      tr(
                        'У включённых провайдеров нет моделей.',
                        'Enabled providers have no models.',
                      ),
                    ),
                  for (final provider in _providers.keys)
                    if (models.any((model) => model.provider == provider)) ...[
                      Padding(
                        padding: const EdgeInsets.only(top: 12, bottom: 4),
                        child: Text(
                          _providers[provider]!,
                          style: Theme.of(context).textTheme.titleSmall,
                        ),
                      ),
                      for (final model in models.where(
                        (model) => model.provider == provider,
                      ))
                        _ModelTile(model: model),
                    ],
                ],
              ),
            ),
          ],
        );
      },
    );
  }
}

class _ModelTile extends StatelessWidget {
  const _ModelTile({required this.model});
  final SavedModelInfo model;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      contentPadding: const EdgeInsets.symmetric(horizontal: 8),
      leading: const Icon(Icons.smart_toy_outlined),
      title: Row(
        children: [
          Flexible(child: Text(model.name)),
          if (model.builtin) ...[
            const SizedBox(width: 6),
            Tooltip(
              message: tr(
                'Встроенная модель: поставляется с приложением, изменить или удалить её нельзя',
                'Built-in model: included with the app and cannot be edited or deleted',
              ),
              child: Icon(Icons.lock_outline, size: 16, color: Theme.of(context).hintColor),
            ),
          ],
        ],
      ),
      subtitle: Text(modelSpec(model)),
      trailing: Wrap(
        spacing: 4,
        children: [
          if (!model.builtin)
            IconButton(
              tooltip: tr('Изменить', 'Edit'),
              onPressed: () => _editModel(context, model),
              icon: const Icon(Icons.edit_outlined),
            ),
          if (!model.builtin)
            IconButton(
              tooltip: tr('Удалить', 'Delete'),
              onPressed: () => _deleteModel(context, model),
              icon: const Icon(Icons.delete_outline),
            ),
        ],
      ),
    );
  }
}

Future<void> _editModel(
  BuildContext context, [
  SavedModelInfo? existing,
]) async {
  final settings = context.appServices.settingsService.provider;
  final enabledProviders = _providers.keys
      .where(settings.providerEnabled)
      .toList();
  if (enabledProviders.isEmpty) {
    return;
  }
  final name = TextEditingController(text: existing?.name ?? '');
  final model = TextEditingController(text: existing?.model ?? '');
  var provider = existing?.provider ?? enabledProviders.first;
  String? reasoning = existing?.reasoning;
  final save = await showDialog<bool>(
    context: context,
    builder: (context) => StatefulBuilder(
      builder: (context, setState) => AlertDialog(
        title: Text(
          existing == null
              ? tr('Добавить модель', 'Add model')
              : tr('Изменить модель', 'Edit model'),
        ),
        content: SizedBox(
          width: 520,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              TextField(
                controller: name,
                autofocus: true,
                decoration: InputDecoration(labelText: tr('Название', 'Name')),
              ),
              const SizedBox(height: 12),
              DropdownButtonFormField<String>(
                initialValue: provider,
                decoration: InputDecoration(
                  labelText: tr('Провайдер', 'Provider'),
                ),
                items: [
                  for (final value in enabledProviders)
                    DropdownMenuItem(
                      value: value,
                      child: Text(_providers[value]!),
                    ),
                ],
                onChanged: (value) =>
                    setState(() => provider = value ?? provider),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: model,
                decoration: const InputDecoration(
                  labelText: 'Model ID',
                  hintText: 'claude-opus-6, gpt-6, llama3.3',
                ),
              ),
              const SizedBox(height: 12),
              DropdownButtonFormField<String?>(
                initialValue: reasoning,
                decoration: InputDecoration(labelText: tr('Усилие', 'Effort')),
                items: [
                  DropdownMenuItem(
                    value: null,
                    child: Text(tr('по умолчанию', 'default')),
                  ),
                  for (final effort in settings.efforts)
                    DropdownMenuItem(value: effort, child: Text(effort)),
                ],
                onChanged: (value) => setState(() => reasoning = value),
              ),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: Text(tr('Отмена', 'Cancel')),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: Text(tr('Сохранить', 'Save')),
          ),
        ],
      ),
    ),
  );
  if (save == true && context.mounted) {
    context.appServices.modelsService.save(
      id: existing?.id,
      name: name.text.trim(),
      provider: provider,
      model: model.text.trim(),
      reasoning: reasoning,
    );
  }
  name.dispose();
  model.dispose();
}

Future<void> _deleteModel(BuildContext context, SavedModelInfo model) async {
  final confirmed = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(tr('Удалить модель?', 'Delete model?')),
      content: Text(model.name),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: Text(tr('Отмена', 'Cancel')),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(context, true),
          child: Text(tr('Удалить', 'Delete')),
        ),
      ],
    ),
  );
  if (confirmed == true && context.mounted) {
    context.appServices.modelsService.delete(model.id);
  }
}
