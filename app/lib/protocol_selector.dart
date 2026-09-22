import 'package:flutter/material.dart';

class ProtocolSelector extends StatelessWidget {
  const ProtocolSelector({super.key, required this.value, this.onChanged});

  final String value;
  final ValueChanged<String>? onChanged;

  @override
  Widget build(BuildContext context) => PopupMenuButton<String>(
    tooltip: '选择协议',
    position: PopupMenuPosition.under,
    enabled: onChanged != null,
    borderRadius: BorderRadius.circular(12),
    onSelected: onChanged,
    itemBuilder: (_) => [
      for (final protocol in ['HTTP', 'HTTPS', 'SOCKS4', 'SOCKS5'])
        PopupMenuItem(value: protocol, child: Text(protocol)),
    ],
    child: InputDecorator(
      decoration: InputDecoration(enabled: onChanged != null),
      child: Row(
        children: [
          Expanded(
            child: Text(
              value,
              style: onChanged == null
                  ? TextStyle(color: Theme.of(context).disabledColor)
                  : null,
            ),
          ),
          const Icon(Icons.arrow_drop_down),
        ],
      ),
    ),
  );
}
