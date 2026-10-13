# Macros

Nerthus allows you to encapsulate a graph inside a node - this is known as a Macro. You can expose parameters of nodes inside the graph. You can add arithmetic and logic in each parameter as well.

## Features

* Expose variables/parameters, and bind one to many properties.
* Use Expressions to filter incoming values.
* Add optional inputs and outputs.
* Secure export to protect your source graph.
* Brand your macro/nodes, if you prefer.

## Comparison to World Machine

WM keeps entire macro worlds within a macro node, so each usage of a Macro node adds 1x the weight. Nerthus references them from a central library and creates instances instead of copies. This is similar to linking to a DLL. 

* The Macro is efficiently packaged into a lightweight version for fast execution. 
* 20 copies of the same macro in a single world only create 1 node's worth of overhead as they share the same definition.
* Updating the macro in a project updates all instances. Great for upgrades, bug fixes, etc.


