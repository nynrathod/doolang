use crate::codegen::core::CodeGen;

use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_method_call(
        &mut self,
        dest: &str,
        object: &str,
        method: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        let object_val = self.resolve_value(object);

        // Check arrays and maps BEFORE strings, since they are also pointer types
        if self.heap_arrays.contains(object) || self.array_metadata.contains_key(object) {
            self.generate_array_method(dest, object, object_val, method, args)
        } else if self.heap_maps.contains(object) || self.map_metadata.contains_key(object) {
            self.generate_map_method(dest, object, object_val, method, args)
        } else if self.heap_strings.contains(object)
            || self.temp_strings.contains_key(object)
            || object_val.is_pointer_value()
        {
            self.generate_string_method(dest, object, object_val, method, args)
        } else {
            None
        }
    }
}
